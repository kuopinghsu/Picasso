//! Publish to WordPress…: the open Story saved as a draft on a connected
//! site (`docs/TRAVEL.md`). The dialog shows where it goes, the photos'
//! size, how many there are and about how large, and asks for approval
//! before anything is uploaded; the job runs in `src/publish.rs`.

use eframe::egui::{self, Margin, RichText};

use crate::db::{Publication, Site};
use crate::i18n::{tr, trf};
use crate::publish::{self, Event, Finished, Job, Stopped};
use crate::story_html::{self, Preflight};
use crate::types::{format_bytes, unix_now};
use crate::upload::UploadSize;
use crate::wordpress::{Api, Credentials, Network, Policy, SiteUrl};
use crate::worker::UiEvent;

use super::publishing::failure_text;
use super::settings::local_time;
use super::{about, theme, Modal, PicassoApp};

/// The choices of the dialog.
pub(super) struct PublishDialog {
    site_id: Option<i64>,
    size: UploadSize,
    /// The person read where the photos go and agreed to upload them.
    approved: bool,
    /// For a post no longer a draft on the site: whether to change it
    /// (`true`, the default) or save a new draft (`false`).
    update: Option<bool>,
    check: Preflight,
    /// Each photo's width and height, for the estimate.
    dimensions: Vec<(u32, u32)>,
    error: String,
}

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
        self.modal = Modal::PublishStory(PublishDialog { site_id, size, approved: false, update: Some(true), check, dimensions, error: String::new() });
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
                ui.horizontal(|ui| {
                    ui.label(tr("Site"));
                    let shown = site.as_ref().map_or_else(|| tr("Choose a site").to_string(), |site| site.label.clone());
                    egui::ComboBox::from_id_salt("publish-site").selected_text(shown).width(300.0).show_ui(ui, |ui| {
                        for option in &sites {
                            let label = if usable(option) { option.label.clone() } else { format!("{} ({})", option.label, tr("can't publish")) };
                            if ui.add_enabled(usable(option), egui::Button::selectable(dialog.site_id == Some(option.id), label)).clicked() {
                                dialog.site_id = Some(option.id);
                                dialog.approved = false;
                            }
                        }
                    });
                });
                if let Some(site) = &site {
                    let who = if site.account.is_empty() { site.user.clone() } else { site.account.clone() };
                    note(ui, &format!("{} · {who}", site.site_url));
                }
                match &earlier {
                    Some(publication) if publication.remote_status != "draft" => {
                        ui.label(trf("The post saved there on {} is no longer a draft ({}).", &[&local_time(publication.synced_at), &publication.remote_status]));
                        if ui.radio(dialog.update == Some(true), tr("Update that post")).clicked() {
                            dialog.update = Some(true);
                            dialog.approved = false;
                        }
                        ui.indent("update-post", |ui| note(ui, tr("Its status stays as it is; if it is published, readers see the changes at once.")));
                        if ui.radio(dialog.update == Some(false), tr("Save as a new draft")).clicked() {
                            dialog.update = Some(false);
                            dialog.approved = false;
                        }
                        ui.indent("new-draft", |ui| note(ui, tr("The post stays as it is; a new draft is made beside it.")));
                    }
                    Some(publication) => note(ui, &trf("Changes the draft saved there on {}. Picasso checks it is still a draft first.", &[&local_time(publication.synced_at)])),
                    None => note(ui, tr("Makes a new draft there. Nothing is published.")),
                }
                let published = earlier.as_ref().is_some_and(|publication| publication.remote_status != "draft");

                ui.add_space(4.0);
                ui.label(RichText::new(tr("Photo size")).strong());
                for size in UploadSize::ALL {
                    let (name, about) = match size {
                        UploadSize::Standard => (tr("Standard web"), tr("Up to 2048 pixels on the longest side, JPEG quality 85")),
                        UploadSize::Low => (tr("Low resolution"), tr("Up to 1280 pixels on the longest side, JPEG quality 80. Smaller files; photos show less detail on large screens.")),
                    };
                    if ui.radio_value(&mut dialog.size, size, name).changed() {
                        dialog.approved = false;
                    }
                    ui.indent(size.code(), |ui| note(ui, about));
                }
                if earlier.as_ref().is_some_and(|publication| publication.size == UploadSize::Standard.code()) && dialog.size == UploadSize::Low {
                    note(ui, tr("Photos uploaded before at the larger size stay on the site. The smaller size applies to new uploads only."));
                }
                let estimate = crate::upload::estimated_bytes(&dialog.dimensions, dialog.size);
                ui.label(trf("{} photos, about {} before upload", &[&dialog.check.photos, &format_bytes(estimate)]));
                note(ui, tr("Photos are rendered from your edits, without camera details or location. Photos already on the site in the same size are not sent again."));

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

                ui.add_space(4.0);
                if let Some(site) = &site {
                    egui::Frame::new().fill(theme::tray()).corner_radius(6).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(trf("The photos are uploaded to {} before the draft is made. Uploaded photos can be reached by their address even while the post is a draft.", &[&site.label])).size(12.5))
                                .wrap(),
                        );
                        let agree = if published && dialog.update == Some(true) { tr("Upload these photos and update the post") } else { tr("Upload these photos and save the draft") };
                        if ui.checkbox(&mut dialog.approved, agree).changed() {
                            dialog.error.clear();
                        }
                    });
                }
                if !dialog.error.is_empty() {
                    ui.label(RichText::new(&dialog.error).color(theme::DANGER));
                }
                ui.horizontal(|ui| {
                    let ready = site.as_ref().is_some_and(usable) && dialog.approved && !blocked && (!published || dialog.update.is_some());
                    let action = if published && dialog.update == Some(true) { tr("Update post") } else { tr("Save draft") };
                    start = ui.add_enabled(ready, theme::primary_button(action)).clicked();
                    close |= ui.button(tr("Cancel")).clicked();
                });
            });
        });
        if settings {
            self.modal = Modal::None;
            self.open_settings(ctx);
            return;
        }
        if close || response.should_close() {
            self.modal = Modal::None;
            return;
        }
        let Some(site) = site.filter(|_| start) else { return };
        let size = dialog.size;
        // A new draft beside a published post replaces the record of it here, never the post.
        let update = dialog.update == Some(true);
        let earlier = earlier.filter(|publication| publication.remote_status == "draft" || update);
        match self.start_publishing(ctx, &site, size, earlier, update) {
            Ok(()) => {
                self.modal = Modal::None;
                self.config.set("publish_site", site.id as f64);
                self.config.set("publish_size", size.code());
            }
            Err(err) => {
                if let Modal::PublishStory(dialog) = &mut self.modal {
                    dialog.error = err;
                }
            }
        }
    }

    fn start_publishing(&mut self, ctx: &egui::Context, site: &Site, size: UploadSize, earlier: Option<Publication>, update: bool) -> Result<(), String> {
        let password = match self.vault.load(&site.secret_ref) {
            Ok(Some(password)) => password,
            Ok(None) => return Err(tr("Picasso doesn't have this site's password any more. Use Edit… in Settings → Publishing to enter it again.").into()),
            Err(_) => return Err(tr("This computer's password store couldn't be used.").into()),
        };
        let (Ok(site_url), Ok(root)) = (SiteUrl::parse(&site.site_url), SiteUrl::parse(&site.api_root)) else {
            return Err(failure_text(&crate::wordpress::Failure::InvalidUrl));
        };
        if !self.save_story() {
            return Err(tr("The story couldn't be saved").into());
        }
        let Some(page) = self.story_job() else { return Err(tr("The story couldn't be saved").into()) };
        let development = site_url.scheme == "http";
        let job = Job {
            api: Api { site: site_url, root, name: site.label.clone(), namespaces: vec!["wp/v2".into()] },
            credentials: Credentials { user: site.user.clone(), password },
            policy: Policy { development },
            story: page.story,
            blocks: page.blocks,
            sources: page.sources,
            size,
            uploads: self.db.uploads(site.id),
            post_id: earlier.map(|publication| publication.post_id),
            map: Some(page.map),
            profiles: page.profiles,
            update_published: update,
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
                let publication = Publication { story_id, site_id, post_id: saved.post_id, remote_status: saved.status.clone(), size: size.code().into(), revision, synced_at: unix_now() };
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
