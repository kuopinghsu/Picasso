//! Publish to WordPress…: the open Story saved as a draft on a connected
//! site (`docs/TRAVEL.md`). The dialog shows where it goes, the photos'
//! size, how many there are and about how large, the categories and tags
//! chosen from the site's own, and asks for approval before anything is
//! uploaded; the job runs in `src/publish.rs`.

use crossbeam_channel::Receiver;
use eframe::egui::{self, Margin, RichText};

use crate::db::{PostTerms, Publication, Site};
use crate::i18n::{tr, trf};
use crate::publish::{self, Event, Finished, Job, Stopped};
use crate::story_html::{self, Preflight};
use crate::types::{format_bytes, unix_now};
use crate::upload::UploadSize;
use crate::wordpress::{edit_link, Api, Client, Credentials, Failure, Network, Policy, SiteUrl, Taxonomy, Term};
use crate::worker::UiEvent;

use super::publishing::failure_text;
use super::settings::local_time;
use super::{about, theme, Modal, PicassoApp};

/// The choices of the dialog.
pub(super) struct PublishDialog {
    site_id: Option<i64>,
    size: UploadSize,
    /// Whether photos keep their camera details (`upload::KEPT`).
    exif: bool,
    /// For a post no longer a draft on the site: whether to change it
    /// (`true`, the default) or save a new draft (`false`).
    update: Option<bool>,
    check: Preflight,
    /// Each photo's width and height, for the estimate.
    dimensions: Vec<(u32, u32)>,
    /// The categories and tags chosen for the post, and the site they were read for.
    terms: PostTerms,
    terms_site: Option<i64>,
    site_terms: SiteTerms,
    /// The site's categories and tags are open over the dialog.
    picking: bool,
    error: String,
}

/// The site's own categories and tags, read when the person asks.
#[derive(Default)]
struct SiteTerms {
    reading: Option<Receiver<Result<[Vec<Term>; 2], Failure>>>,
    /// Categories, then tags, by name.
    found: Option<[Vec<Term>; 2]>,
    error: String,
    filter: String,
}

/// Tags listed at once; typing narrows a longer list.
const SHOWN_TAGS: usize = 60;

impl PicassoApp {
    pub(super) fn ask_publish_story(&mut self) {
        let Some(job) = self.story_job() else { return };
        let check = story_html::preflight(&job.page());
        let mut dimensions = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for &media_id in job.sources.keys() {
            if let Some(photo) = self.story_photo(media_id).filter(|photo| seen.insert(photo.id)) {
                // The catalog's quarter turns swap the sides; the estimate doesn't mind.
                dimensions.push((photo.width, photo.height));
            }
        }
        let sites = self.db.sites();
        let remembered = self.config.number("publish_site").map(|id| id as i64);
        let site_id = remembered.filter(|id| sites.iter().any(|site| site.id == *id)).or_else(|| sites.iter().find(|site| usable(site)).map(|site| site.id));
        let size = self.publish_size(job.story.id, site_id);
        // A post edited on the site is replaced only when that is chosen.
        let changed = site_id.and_then(|site_id| self.db.publication(job.story.id, site_id)).is_some_and(|publication| publication.changed_on_site);
        self.modal = Modal::PublishStory(PublishDialog {
            site_id,
            size,
            exif: self.config.flag("publish_exif").unwrap_or(true),
            update: (!changed).then_some(true),
            check,
            dimensions,
            terms: PostTerms::default(),
            terms_site: None,
            site_terms: SiteTerms::default(),
            picking: false,
            error: String::new(),
        });
    }

    /// The size this Story was last saved to the site in, or the last one chosen.
    fn publish_size(&self, story_id: i64, site_id: Option<i64>) -> UploadSize {
        site_id
            .and_then(|site_id| self.db.publication(story_id, site_id))
            .and_then(|publication| UploadSize::from_code(&publication.size))
            .or_else(|| self.config.text("publish_size").and_then(UploadSize::from_code))
            .unwrap_or_default()
    }

    pub(super) fn publish_story_modal(&mut self, ctx: &egui::Context) {
        let story_id = self.story.as_ref().map(|editor| editor.story.id);
        let sites = self.db.sites();
        let Modal::PublishStory(dialog) = &mut self.modal else { return };
        let Some(story_id) = story_id else {
            self.modal = Modal::None;
            return;
        };
        let mut close = false;
        let mut start = false;
        let mut settings = false;
        let site = dialog.site_id.and_then(|id| sites.iter().find(|site| site.id == id)).cloned();
        let earlier = site.as_ref().and_then(|site| self.db.publication(story_id, site.id));
        // Terms belong to one site: another site starts from what was chosen for it.
        if dialog.terms_site != dialog.site_id {
            dialog.terms = dialog.site_id.map(|site_id| self.db.post_terms(story_id, site_id)).unwrap_or_default();
            dialog.terms_site = dialog.site_id;
            dialog.site_terms = SiteTerms::default();
        }
        if let Some(Ok(result)) = dialog.site_terms.reading.as_ref().map(Receiver::try_recv) {
            dialog.site_terms.reading = None;
            match result {
                Ok(found) => dialog.site_terms.found = Some(found),
                Err(failure) => dialog.site_terms.error = failure_text(&failure),
            }
        } else if dialog.site_terms.reading.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let mut read_terms = false;
        let mut choose_terms = false;
        let response = egui::Modal::new(egui::Id::unique("publish-story")).frame(about::dialog_frame()).show(ctx, |ui| {
            ui.set_width(460.0);
            about::title_bar(ui, &self.logo, tr("Publish to WordPress"), &mut close);
            egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                if sites.is_empty() {
                    ui.label(tr("Connect a WordPress site first, in Settings → Publishing."));
                    ui.horizontal(|ui| {
                        settings = ui.add(theme::primary_button(tr("Open Settings"))).clicked();
                        close |= ui.button(tr("Cancel")).clicked();
                    });
                    return;
                }
                let published = earlier.as_ref().is_some_and(|publication| publication.remote_status != "draft");
                let changed = earlier.as_ref().is_some_and(|publication| publication.changed_on_site);
                // One row per choice; what each means is in its tooltip.
                egui::Grid::new("publish-choices").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                    ui.label(RichText::new(tr("Site")).color(theme::muted()));
                    let shown = site.as_ref().map_or_else(|| tr("Choose a site").to_string(), |site| site.label.clone());
                    let combo = egui::ComboBox::from_id_salt("publish-site").selected_text(shown).width(300.0).show_ui(ui, |ui| {
                        for option in &sites {
                            let label = if usable(option) { option.label.clone() } else { format!("{} ({})", option.label, tr("can't publish")) };
                            if ui.add_enabled(usable(option), egui::Button::selectable(dialog.site_id == Some(option.id), label)).clicked() {
                                dialog.site_id = Some(option.id);
                            }
                        }
                    });
                    if let Some(site) = &site {
                        let who = if site.account.is_empty() { site.user.clone() } else { site.account.clone() };
                        combo.response.on_hover_text(format!("{} · {who}", site.site_url));
                    }
                    ui.end_row();

                    ui.label(RichText::new(tr("Post")).color(theme::muted()));
                    match &earlier {
                        Some(publication) if published || changed => {
                            let (replace, replace_about) = if changed {
                                (tr("Replace the edits made there"), tr("The post becomes this story again; what was changed in it on the site is lost. Its status stays as it is."))
                            } else {
                                (tr("Update that post"), tr("Its status stays as it is; if it is published, readers see the changes at once."))
                            };
                            let shown = match dialog.update {
                                Some(true) => replace,
                                Some(false) => tr("Save as a new draft"),
                                None => tr("Choose…"),
                            };
                            ui.vertical(|ui| {
                                egui::ComboBox::from_id_salt("publish-update").selected_text(shown).width(300.0).show_ui(ui, |ui| {
                                    for (update, label, about) in [(true, replace, replace_about), (false, tr("Save as a new draft"), tr("The post stays as it is; a new draft is made beside it."))] {
                                        if ui.add(egui::Button::selectable(dialog.update == Some(update), label)).on_hover_text(about).clicked() {
                                            dialog.update = Some(update);
                                        }
                                    }
                                });
                                if changed {
                                    ui.horizontal_wrapped(|ui| {
                                        note(ui, &trf("The post was edited on the site after Picasso saved it on {}.", &[&local_time(publication.synced_at)]));
                                        if let Some(link) = site.as_ref().and_then(|site| post_edit_link(site, publication.post_id)) {
                                            if ui.link(tr("Open in WordPress")).clicked() {
                                                let _ = open::that(link);
                                            }
                                        }
                                    });
                                } else {
                                    note(ui, &trf("The post saved there on {} is no longer a draft ({}).", &[&local_time(publication.synced_at), &publication.remote_status]));
                                }
                            });
                        }
                        Some(publication) => {
                            ui.add(egui::Label::new(trf("Changes the draft saved there on {}. Picasso checks it is still a draft first.", &[&local_time(publication.synced_at)])).wrap());
                        }
                        None => {
                            ui.label(tr("Makes a new draft there. Nothing is published."));
                        }
                    }
                    ui.end_row();

                    ui.label(RichText::new(tr("Categories and tags")).color(theme::muted()));
                    ui.horizontal(|ui| {
                        let names: Vec<String> = dialog.terms.categories.iter().map(|(_, name)| name.clone()).chain(dialog.terms.tags.iter().map(|(_, name)| format!("#{name}"))).collect();
                        let none = if earlier.is_some() { tr("None chosen: the post keeps the categories and tags it has on the site.") } else { tr("None chosen: the post goes in the site's default category.") };
                        if names.is_empty() {
                            ui.label(RichText::new(tr("Not chosen")).color(theme::muted())).on_hover_text(none);
                        } else {
                            ui.add(egui::Label::new(names.join(", ")).truncate()).on_hover_text(tr("Chosen ones replace the post's categories or tags on the site. New terms are made on the site itself."));
                        }
                        choose_terms = ui.add_enabled(site.is_some(), egui::Button::new(tr("Choose…")).small()).clicked();
                    });
                    ui.end_row();

                    ui.label(RichText::new(tr("Photo size")).color(theme::muted()))
                        .on_hover_text(tr("Photos are rendered from your edits, never with their location. Photos already on the site in the same size are not sent again."));
                    ui.vertical(|ui| {
                        let name = |size| match size {
                            UploadSize::Standard => (tr("Standard web"), tr("Up to 2048 pixels on the longest side, JPEG quality 85")),
                            UploadSize::Low => (tr("Low resolution"), tr("Up to 1280 pixels on the longest side, JPEG quality 80. Smaller files; photos show less detail on large screens.")),
                        };
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("publish-size").selected_text(name(dialog.size).0).show_ui(ui, |ui| {
                                for size in UploadSize::ALL {
                                    let (label, about) = name(size);
                                    if ui.add(egui::Button::selectable(dialog.size == size, label)).on_hover_text(about).clicked() {
                                        dialog.size = size;
                                    }
                                }
                            });
                            let estimate = crate::upload::estimated_bytes(&dialog.dimensions, dialog.size);
                            ui.label(RichText::new(trf("{} photos, about {} before upload", &[&dialog.check.photos, &format_bytes(estimate)])).color(theme::muted()));
                        });
                        if earlier.as_ref().is_some_and(|publication| publication.size == UploadSize::Standard.code()) && dialog.size == UploadSize::Low {
                            note(ui, tr("Photos uploaded before at the larger size stay on the site. The smaller size applies to new uploads only."));
                        }
                        ui.checkbox(&mut dialog.exif, tr("Keep camera details")).on_hover_text(tr("Camera and lens, focal length, aperture, shutter speed, ISO, and when the photo was taken. Never the location or serial numbers."));
                    });
                    ui.end_row();
                });

                let check = &dialog.check;
                let mut blocked = check.empty;
                if check.empty {
                    ui.label(RichText::new(tr("There is nothing in this story to publish yet.")).color(theme::DANGER));
                }
                if check.missing + check.unreadable > 0 {
                    blocked = true;
                    ui.label(
                        RichText::new(trf("{} photos are unavailable or can't be read. Remove them from the story or bring their files back; nothing is left out of a post.", &[&(check.missing + check.unreadable)]))
                            .color(theme::DANGER),
                    );
                }
                if check.unknown > 0 {
                    blocked = true;
                    ui.label(RichText::new(trf("{} blocks from a newer Picasso can't be published.", &[&check.unknown])).color(theme::DANGER));
                }

                // The button is the approval: it names what is sent, and this says where.
                if let Some(site) = &site {
                    note(ui, &trf("Photos are uploaded to {} first; they can be opened by their address even while the post is a draft.", &[&site.label]));
                }
                if !dialog.error.is_empty() {
                    ui.label(RichText::new(&dialog.error).color(theme::DANGER));
                }
                ui.horizontal(|ui| {
                    let ready = site.as_ref().is_some_and(usable) && !blocked && (!(published || changed) || dialog.update.is_some());
                    let photos = dialog.check.photos;
                    let update = (published || changed) && dialog.update == Some(true);
                    let action = match (photos, update) {
                        (0, true) => tr("Update post").to_string(),
                        (0, false) => tr("Save draft").to_string(),
                        (_, true) => trf("Upload {} photos and update the post", &[&photos]),
                        (_, false) => trf("Upload {} photos and save the draft", &[&photos]),
                    };
                    start = ui.add_enabled(ready, theme::primary_button(&action)).clicked();
                    close |= ui.button(tr("Cancel")).clicked();
                });
            });
        });
        if choose_terms {
            dialog.picking = true;
            read_terms = dialog.site_terms.found.is_none() && dialog.site_terms.reading.is_none();
        }
        // The site's categories and tags open over the dialog, so the dialog stays short.
        if dialog.picking {
            let mut done = false;
            egui::Modal::new(egui::Id::unique("publish-terms")).frame(about::dialog_frame()).show(ctx, |ui| {
                ui.set_width(420.0);
                about::title_bar(ui, &self.logo, tr("Categories and tags"), &mut done);
                egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    read_terms |= terms_picker(ui, &mut dialog.terms, &mut dialog.site_terms, earlier.is_some());
                    ui.horizontal(|ui| done |= ui.add(theme::primary_button(tr("Done"))).clicked());
                });
            });
            if done {
                dialog.picking = false;
            }
        }
        if settings {
            self.modal = Modal::None;
            self.open_settings(ctx);
            return;
        }
        if let Some(site) = site.as_ref().filter(|_| read_terms) {
            match connection(&self.vault, site) {
                Ok((api, credentials, policy)) => {
                    let (sender, receiver) = crossbeam_channel::bounded(1);
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let network = Network::new();
                        let client = Client::new(&network, policy);
                        let read = |taxonomy| client.terms(&api, &credentials, taxonomy);
                        let _ = sender.send(read(Taxonomy::Categories).and_then(|categories| Ok([categories, read(Taxonomy::Tags)?])));
                        ctx.request_repaint();
                    });
                    dialog.site_terms = SiteTerms { reading: Some(receiver), filter: std::mem::take(&mut dialog.site_terms.filter), ..SiteTerms::default() };
                }
                Err(err) => dialog.site_terms.error = err,
            }
        }
        if close || response.should_close() {
            self.modal = Modal::None;
            return;
        }
        let Some(site) = site.filter(|_| start) else { return };
        let size = dialog.size;
        let exif = dialog.exif;
        let terms = dialog.terms.clone();
        // A new draft beside a published post replaces the record of it here, never the post.
        let update = dialog.update == Some(true);
        let earlier = earlier.filter(|publication| (publication.remote_status == "draft" && !publication.changed_on_site) || update);
        match self.start_publishing(ctx, &site, size, exif, terms, earlier, update) {
            Ok(()) => {
                self.modal = Modal::None;
                self.config.set("publish_site", site.id as f64);
                self.config.set("publish_size", size.code());
                self.config.set("publish_exif", exif);
            }
            Err(err) => {
                if let Modal::PublishStory(dialog) = &mut self.modal {
                    dialog.error = err;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_publishing(&mut self, ctx: &egui::Context, site: &Site, size: UploadSize, exif: bool, terms: PostTerms, earlier: Option<Publication>, update: bool) -> Result<(), String> {
        let (api, credentials, policy) = connection(&self.vault, site)?;
        if !self.save_story() {
            return Err(tr("The story couldn't be saved").into());
        }
        let Some(page) = self.story_job() else { return Err(tr("The story couldn't be saved").into()) };
        self.db.save_post_terms(page.story.id, site.id, &terms)?;
        let job = Job {
            api,
            credentials,
            policy,
            story: page.story,
            blocks: page.blocks,
            sources: page.sources,
            size,
            exif,
            uploads: self.db.uploads(site.id),
            post_id: earlier.as_ref().map(|publication| publication.post_id),
            fingerprint: earlier.as_ref().map(|publication| publication.fingerprint.clone()).unwrap_or_default(),
            overwrite: update && earlier.as_ref().is_some_and(|publication| publication.changed_on_site),
            map: Some(page.map),
            profiles: page.profiles,
            update_published: update && earlier.as_ref().is_some_and(|publication| publication.remote_status != "draft"),
            terms,
        };
        let progress = self.start_story_job();
        let (story_id, site_id, revision, label) = (job.story.id, site.id, job.story.revision, site.label.clone());
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-publish".into()).spawn(move || {
            let network = Network::new();
            let outcome = publish::run(&network, &job, &progress, &mut |event| {
                let _ = events.send(UiEvent::PublishProgress(site_id, event));
                ctx.request_repaint();
            });
            let finished = Finished { story_id, site_id, site: label, size, revision, outcome };
            let _ = events.send(UiEvent::StoryPublished(Box::new(finished)));
            ctx.request_repaint();
        });
        Ok(())
    }

    /// Keeps what the job learned the moment it did, so a retry reuses it.
    pub(super) fn on_publish_progress(&mut self, site_id: i64, event: Event) {
        let kept = match event {
            Event::Uploaded { hash, size, media_id, url } => self.db.record_upload(site_id, &hash, size.code(), media_id, &url),
            Event::Gone { hash } => self.db.forget_upload(site_id, &hash),
        };
        if let Err(err) = kept {
            self.activity = trf("Couldn't record an upload: {}", &[&err]);
        }
    }

    pub(super) fn on_story_published(&mut self, finished: Finished) {
        self.end_story_job(finished.story_id);
        let Finished { story_id, site_id, site, size, revision, outcome } = finished;
        self.activity = match outcome {
            Ok(saved) => {
                let publication = Publication {
                    story_id,
                    site_id,
                    post_id: saved.post_id,
                    remote_status: saved.status.clone(),
                    size: size.code().into(),
                    revision,
                    synced_at: unix_now(),
                    fingerprint: saved.fingerprint.clone(),
                    changed_on_site: false,
                };
                if let Err(err) = self.db.save_publication(&publication) {
                    trf("The draft was saved on {}, but Picasso couldn't record it: {}", &[&site, &err])
                } else {
                    let _ = open::that(&saved.edit_link);
                    if saved.status == "draft" {
                        trf("Saved as a draft on {}: {} photos uploaded, {} already there. It opens in WordPress, where you can sign in to see it.", &[&site, &saved.uploaded, &saved.reused])
                    } else {
                        trf("Updated the post on {}: {} photos uploaded, {} already there.", &[&site, &saved.uploaded, &saved.reused])
                    }
                }
            }
            Err(Stopped::Cancelled { uploaded }) => trf("Publishing cancelled. The draft wasn't changed; {} photos that reached the site stay there and are reused next time.", &[&uploaded]),
            Err(Stopped::Site { failure, uploaded }) if uploaded > 0 => {
                format!("{} {}", failure_text(&failure), trf("{} photos reached the site before it stopped; they stay there and are reused next time.", &[&uploaded]))
            }
            Err(Stopped::Site { failure, .. }) => failure_text(&failure),
            Err(Stopped::Photo { .. }) => tr("A photo became unavailable or couldn't be read. The draft wasn't changed; nothing is left out of a post.").into(),
            Err(Stopped::Unsupported(_)) => tr("This story has blocks from a newer Picasso, which can't be published.").into(),
            Err(Stopped::NotDraft(status)) => {
                if status == "trash" {
                    // Never changed; the next save makes a new draft.
                    let _ = self.db.forget_publication(story_id, site_id);
                    trf("The post on {} is in the trash. Picasso leaves it alone; publish again to make a new draft.", &[&site])
                } else {
                    // Kept, so the dialog offers to update it or save a new draft.
                    if let Some(mut publication) = self.db.publication(story_id, site_id) {
                        publication.remote_status = status.clone();
                        let _ = self.db.save_publication(&publication);
                    }
                    trf("The post on {} isn't a draft any more ({}). Nothing was changed; publish again to update it or to save a new draft.", &[&site, &status])
                }
            }
            Err(Stopped::ChangedOnSite) => {
                // Kept, so the dialog offers to replace the edits or save a new draft.
                if let Some(mut publication) = self.db.publication(story_id, site_id) {
                    publication.changed_on_site = true;
                    let _ = self.db.save_publication(&publication);
                }
                trf("The post on {} was edited there since Picasso saved it. Nothing was changed; publish again to choose whether to replace those edits or save a new draft.", &[&site])
            }
            Err(Stopped::PostGone) => {
                let _ = self.db.forget_publication(story_id, site_id);
                trf("The earlier draft is gone from {}. Save the draft again to make a new one.", &[&site])
            }
        };
    }
}

/// Whether the account may upload photos and write posts there.
fn usable(site: &Site) -> bool {
    site.can_upload && site.can_edit
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::muted())).wrap());
}

/// The site's API and the account's credentials from the password store.
fn connection(vault: &crate::keychain::Vault, site: &Site) -> Result<(Api, Credentials, Policy), String> {
    let password = match vault.load(&site.secret_ref) {
        Ok(Some(password)) => password,
        Ok(None) => return Err(tr("Picasso doesn't have this site's password any more. Use Edit… in Settings → Publishing to enter it again.").into()),
        Err(_) => return Err(tr("This computer's password store couldn't be used.").into()),
    };
    let (Ok(site_url), Ok(root)) = (SiteUrl::parse(&site.site_url), SiteUrl::parse(&site.api_root)) else {
        return Err(failure_text(&Failure::InvalidUrl));
    };
    let policy = Policy { development: site_url.scheme == "http" };
    let api = Api { site: site_url, root, name: site.label.clone(), namespaces: vec!["wp/v2".into()] };
    Ok((api, Credentials { user: site.user.clone(), password }, policy))
}

/// Where post `post_id` opens in the site's editor.
fn post_edit_link(site: &Site, post_id: i64) -> Option<String> {
    let (site_url, root) = (SiteUrl::parse(&site.site_url).ok()?, SiteUrl::parse(&site.api_root).ok()?);
    Some(edit_link(&Api { site: site_url, root, name: String::new(), namespaces: Vec::new() }, post_id))
}

/// The site's categories and tags to tick, those chosen shown first so they
/// can be taken off. Returns whether to read the site's again.
fn terms_picker(ui: &mut egui::Ui, chosen: &mut PostTerms, site: &mut SiteTerms, earlier: bool) -> bool {
    let mut read = false;
    if chosen.categories.is_empty() && chosen.tags.is_empty() {
        let none = if earlier { tr("None chosen: the post keeps the categories and tags it has on the site.") } else { tr("None chosen: the post goes in the site's default category.") };
        note(ui, none);
    } else {
        ui.horizontal_wrapped(|ui| {
            for (list, prefix) in [(&mut chosen.categories, ""), (&mut chosen.tags, "#")] {
                list.retain(|(_, name)| !ui.small_button(format!("{prefix}{name}  ×")).on_hover_text(tr("Take off")).clicked());
            }
        });
        note(ui, tr("Chosen ones replace the post's categories or tags on the site. New terms are made on the site itself."));
    }
    if !site.error.is_empty() {
        ui.label(RichText::new(&site.error).color(theme::DANGER));
    }
    if site.reading.is_some() {
        ui.add(egui::Spinner::new().size(14.0));
        return read;
    }
    if site.found.is_none() {
        read = ui.button(tr("Read again")).clicked();
        return read;
    }
    let Some([categories, tags]) = &site.found else { return read };
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut site.filter).hint_text(tr("Find a category or tag")).desired_width(300.0));
        read = ui.small_button(tr("Read again")).clicked();
    });
    let wanted = site.filter.trim().to_lowercase();
    let matches = |term: &&Term| wanted.is_empty() || term.name.to_lowercase().contains(&wanted);
    ui.columns(2, |columns| {
        columns[0].label(RichText::new(tr("Categories")).color(theme::muted()).size(12.0));
        egui::ScrollArea::vertical().max_height(140.0).id_salt("publish-categories").show(&mut columns[0], |ui| {
            let shown: Vec<(usize, &Term)> = by_parent(categories).into_iter().filter(|(_, term)| matches(term)).collect();
            if shown.is_empty() {
                note(ui, tr("None"));
            }
            for (depth, term) in shown {
                // A filtered list shows children on their own, so only a whole list is indented.
                let indent = if wanted.is_empty() { depth as f32 * 14.0 } else { 0.0 };
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    toggle(ui, &mut chosen.categories, term);
                });
            }
        });
        columns[1].label(RichText::new(tr("Tags")).color(theme::muted()).size(12.0));
        egui::ScrollArea::vertical().max_height(140.0).id_salt("publish-tags").show(&mut columns[1], |ui| {
            let shown: Vec<&Term> = tags.iter().filter(matches).collect();
            if shown.is_empty() {
                note(ui, tr("None"));
            }
            for term in shown.iter().take(SHOWN_TAGS) {
                toggle(ui, &mut chosen.tags, term);
            }
            if shown.len() > SHOWN_TAGS {
                note(ui, &trf("{} more; type to find them.", &[&(shown.len() - SHOWN_TAGS)]));
            }
        });
    });
    read
}

/// Categories with each one's children after it, and how deep each is.
/// One whose parent isn't listed is shown at the top.
fn by_parent(categories: &[Term]) -> Vec<(usize, &Term)> {
    let ids: std::collections::HashSet<i64> = categories.iter().map(|term| term.id).collect();
    let mut out = Vec::with_capacity(categories.len());
    fn walk<'t>(all: &'t [Term], parent: i64, depth: usize, out: &mut Vec<(usize, &'t Term)>) {
        // Depth is capped so a loop in bad data can't recurse forever.
        if depth > 8 {
            return;
        }
        for term in all.iter().filter(|term| term.parent == parent && term.id != parent) {
            out.push((depth, term));
            walk(all, term.id, depth + 1, out);
        }
    }
    for root in categories.iter().filter(|term| term.parent == 0 || term.parent == term.id || !ids.contains(&term.parent)) {
        out.push((0, root));
        walk(categories, root.id, 1, &mut out);
    }
    out
}

/// A checkbox that adds `term` to `list` or takes it off.
fn toggle(ui: &mut egui::Ui, list: &mut Vec<(i64, String)>, term: &Term) {
    let mut on = list.iter().any(|(id, _)| *id == term.id);
    if ui.checkbox(&mut on, &term.name).changed() {
        if on {
            list.push((term.id, term.name.clone()));
        } else {
            list.retain(|(id, _)| *id != term.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_list_each_child_after_its_parent() {
        let term = |id, name: &str, parent| Term { id, name: name.into(), parent };
        let categories = [term(2, "Europe", 0), term(6, "Granada", 5), term(5, "Spain", 2), term(1, "Uncategorized", 0), term(9, "Orphan", 77), term(4, "Loop", 4)];
        let listed: Vec<(usize, &str)> = by_parent(&categories).into_iter().map(|(depth, term)| (depth, term.name.as_str())).collect();
        assert_eq!(listed, [(0, "Europe"), (1, "Spain"), (2, "Granada"), (0, "Uncategorized"), (0, "Orphan"), (0, "Loop")]);
    }
}
