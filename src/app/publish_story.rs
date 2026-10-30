//! Publish to WordPress…: the open Story saved as a draft on a connected
//! site (`docs/TRAVEL.md`). The dialog shows where it goes, the photos'
//! size, how many there are and about how large, the categories and tags
//! chosen from the site's own, and asks for approval before anything is
//! uploaded; the job runs in `src/publish.rs`.

use crossbeam_channel::Receiver;
use eframe::egui::{self, Margin, RichText};

use crate::db::{PostTerms, Publication, Site};
use crate::i18n::{tr, trf};
use crate::publish::{self, Event, Finished, Job, Stopped, SyncState};
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
    /// For a post with the Story's title found on the site: update it
    /// (`true`) or make a new one (`false`); asked, never assumed.
    adopt: Option<bool>,
    /// Make the post public once saved, which is confirmed first.
    publish: bool,
    confirming: bool,
    /// Publish as an index post and one per Day, offered when the Story
    /// has Days: its top-level headings.
    by_day: bool,
    days: usize,
    /// Change Day posts edited on the site or already public; asked, never assumed.
    day_replace: bool,
    /// Publish at a set time, typed as the site's date and time.
    at_time: bool,
    date: String,
    time: String,
    /// What the confirmation names: the Story's title and its route maps,
    /// which show where the trip went.
    title: String,
    routes: usize,
    /// Photos with transparent areas, which are laid on white.
    transparent: usize,
    /// The library photos shown as the preview of what is sent, in order.
    previews: Vec<i64>,
    /// For the summary: how the first photo's upload is named, and whether
    /// the Story has a subtitle and a cover.
    sample_name: String,
    excerpt: bool,
    cover: bool,
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
    /// A term being made on the site, and the one asked for next.
    making: Option<Receiver<Result<(Taxonomy, Term), Failure>>>,
    make: Option<(Taxonomy, String)>,
}

/// Tags listed at once; typing narrows a longer list.
const SHOWN_TAGS: usize = 60;

impl PicassoApp {
    pub(super) fn ask_publish_story(&mut self) {
        let Some(job) = self.story_job() else { return };
        let check = story_html::preflight(&job.page());
        let mut dimensions = Vec::new();
        let mut previews = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut order: Vec<i64> = job.sources.keys().copied().collect();
        order.sort_unstable();
        for media_id in order {
            if let Some(photo) = self.story_photo(media_id).filter(|photo| seen.insert(photo.id)) {
                // The catalog's quarter turns swap the sides; the estimate doesn't mind.
                dimensions.push((photo.width, photo.height));
                previews.push(photo.id);
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
            adopt: None,
            publish: false,
            confirming: false,
            by_day: false,
            day_replace: false,
            days: job.blocks.iter().filter(|block| matches!(block.block, crate::trip::Block::Heading(_))).count(),
            at_time: false,
            date: String::new(),
            time: String::new(),
            title: job.story.title.trim().to_string(),
            previews,
            sample_name: {
                let first = job.sources.iter().min_by_key(|(id, _)| **id).and_then(|(_, source)| source.as_ref()).and_then(|(path, ..)| path.file_stem()).map(|stem| stem.to_string_lossy().into_owned());
                publish::upload_name(&job.story.title, first.as_deref().unwrap_or_default(), "00000000").trim_end_matches("00000000").to_string() + "….jpg"
            },
            excerpt: !job.story.subtitle.trim().is_empty(),
            cover: job.story.cover.is_some() || !job.sources.is_empty(),
            transparent: job.sources.values().flatten().filter(|(path, ..)| crate::upload::has_transparency(path)).count(),
            routes: job.blocks.iter().filter(|block| matches!(block.block, crate::trip::Block::Route { .. })).count(),
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
        let revision = self.story.as_ref().map_or(0, |editor| editor.story.revision);
        // The preview's thumbnails, read before the dialog is borrowed.
        let preview_ids = match &self.modal {
            Modal::PublishStory(dialog) => dialog.previews.iter().take(PREVIEW_SHOWN).copied().collect(),
            _ => Vec::new(),
        };
        let previews: Vec<_> = preview_ids.into_iter().filter_map(|id| self.thumb_texture(id)).collect();
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
        let found = match (self.publish_found, &site) {
            (Some((story, site_id, post)), Some(site)) if story == story_id && site_id == site.id && earlier.is_none() => Some(post),
            _ => None,
        };
        // Terms belong to one site: another site starts from what was chosen for it.
        if dialog.terms_site != dialog.site_id {
            dialog.terms = dialog.site_id.map(|site_id| self.db.post_terms(story_id, site_id)).unwrap_or_default();
            dialog.terms_site = dialog.site_id;
            dialog.site_terms = SiteTerms::default();
            // As it went to that site last: by Day when its Day posts were saved after its one post.
            dialog.by_day = dialog.days > 0
                && dialog.site_id.is_some_and(|site_id| {
                    let by_day = self.db.day_posts(story_id, site_id).iter().map(|(_, post)| post.synced_at).max();
                    let single = self.db.publication(story_id, site_id).map(|publication| publication.synced_at);
                    by_day.is_some_and(|by_day| single.is_none_or(|single| by_day >= single))
                });
        }
        if let Some(Ok(result)) = dialog.site_terms.reading.as_ref().map(Receiver::try_recv) {
            dialog.site_terms.reading = None;
            match result {
                Ok(found) => dialog.site_terms.found = Some(found),
                Err(failure) => dialog.site_terms.error = failure_text(&failure),
            }
        } else if let Some(Ok(result)) = dialog.site_terms.making.as_ref().map(Receiver::try_recv) {
            dialog.site_terms.making = None;
            match result {
                Ok((taxonomy, term)) => {
                    let index = usize::from(taxonomy == Taxonomy::Tags);
                    let chosen = if index == 0 { &mut dialog.terms.categories } else { &mut dialog.terms.tags };
                    if !chosen.iter().any(|(id, _)| *id == term.id) {
                        chosen.push((term.id, term.name.clone()));
                    }
                    if let Some(list) = dialog.site_terms.found.as_mut().map(|found| &mut found[index]) {
                        if !list.iter().any(|known| known.id == term.id) {
                            list.push(term);
                            list.sort_by_key(|known| known.name.to_lowercase());
                        }
                    }
                    dialog.site_terms.filter.clear();
                }
                Err(Failure::Forbidden) => dialog.site_terms.error = tr("This account isn't allowed to add categories or tags on the site.").to_string(),
                Err(failure) => dialog.site_terms.error = failure_text(&failure),
            }
        } else if dialog.site_terms.reading.is_some() || dialog.site_terms.making.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let mut read_terms = false;
        let mut choose_terms = false;
        let response = egui::Modal::new(egui::Id::unique("publish-story")).frame(about::dialog_frame()).show(ctx, |ui| {
            // Fixed, so a long row wraps instead of widening the dialog past its title bar.
            ui.set_width(DIALOG_WIDTH);
            ui.set_max_width(DIALOG_WIDTH);
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
                // Whether readers already see it: by Day, the Day posts' own state counts.
                let day_public = site.as_ref().is_some_and(|site| self.db.day_posts(story_id, site.id).iter().any(|(_, post)| post.remote_status != "draft"));
                let public = if dialog.by_day { day_public } else { published || changed };
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

                    ui.label(RichText::new(tr("Post")).color(theme::muted())).on_hover_text(tr(
                        "Picasso checks the post on the site just before saving, but WordPress can't hold it while Picasso saves: an edit made there in those few seconds can be replaced.",
                    ));
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
                                    note(ui, &trf("The post saved there on {} is no longer a draft ({}).", &[&local_time(publication.synced_at), &status_name(&publication.remote_status)]));
                                }
                            });
                        }
                        Some(publication) => {
                            ui.add(egui::Label::new(trf("Changes the draft saved there on {}. Picasso checks it is still a draft first.", &[&local_time(publication.synced_at)])).wrap());
                        }
                        None if found.is_some() => {
                            let shown = match dialog.adopt {
                                Some(true) => tr("Update the post already there"),
                                Some(false) => tr("Make a new post"),
                                None => tr("Choose…"),
                            };
                            ui.vertical(|ui| {
                                egui::ComboBox::from_id_salt("publish-adopt").selected_text(shown).width(300.0).show_ui(ui, |ui| {
                                    for (adopt, label, about) in [
                                        (true, tr("Update the post already there"), tr("Picasso takes it as this story's post and changes it; its status stays as it is.")),
                                        (false, tr("Make a new post"), tr("The post there stays as it is; another is made beside it.")),
                                    ] {
                                        if ui.add(egui::Button::selectable(dialog.adopt == Some(adopt), label)).on_hover_text(about).clicked() {
                                            dialog.adopt = Some(adopt);
                                        }
                                    }
                                });
                                ui.horizontal_wrapped(|ui| {
                                    note(ui, tr("The site has a post with this title that Picasso has no record of, as after restoring an older library."));
                                    if let Some(link) = site.as_ref().zip(found).and_then(|(site, post)| post_edit_link(site, post)) {
                                        if ui.link(tr("Open in WordPress")).clicked() {
                                            let _ = open::that(link);
                                        }
                                    }
                                });
                            });
                        }
                        None => {
                            ui.label(if dialog.publish { tr("Makes a new post there, public once you confirm.") } else { tr("Makes a new draft there. Nothing is published.") });
                        }
                    }
                    ui.end_row();

                    // Where the post stands, apart from its status on the site.
                    if let Some(site) = &site {
                        let pending = earlier.is_none() && self.db.pending_draft(story_id, site.id).is_some();
                        let state = publish::sync_state(earlier.as_ref(), revision, pending);
                        if state != SyncState::New {
                            ui.label(RichText::new(tr("State")).color(theme::muted()));
                            let (text, about) = match state {
                                SyncState::Clean => (tr("Up to date"), tr("The site has this story as it is here.")),
                                SyncState::ChangedHere => (tr("Changed here"), tr("The story was changed here since it was last saved on the site.")),
                                SyncState::Conflict => (tr("Edited on the site"), tr("The post was edited on the site since Picasso saved it; choose above what to do.")),
                                SyncState::Unconfirmed | SyncState::New => (tr("Not confirmed"), tr("A post was being made and the site never confirmed it; Picasso looks for it before making another.")),
                            };
                            ui.label(text).on_hover_text(about);
                            ui.end_row();
                        }
                    }

                    // A Story with Days can go out as an index and a post per Day.
                    ui.label(RichText::new(tr("By Day")).color(theme::muted()));
                    let tick = ui.add_enabled(dialog.days > 0, egui::Checkbox::new(&mut dialog.by_day, trf("An index and one post per Day ({} Days)", &[&dialog.days])));
                    if dialog.days > 0 {
                        tick.on_hover_text(tr("Each top-level heading starts a Day post titled after it. What comes before the first heading goes in the index, with the introduction and conclusion and links to the Days; each Day links back and to its neighbours."));
                    } else {
                        tick.on_disabled_hover_text(tr("This story has no Days: a top-level heading starts each."));
                    }
                    ui.end_row();
                    if dialog.by_day {
                        let known = site.as_ref().map(|site| self.db.day_posts(story_id, site.id)).unwrap_or_default();
                        let touched = known.iter().filter(|(_, post)| post.changed_on_site || post.remote_status != "draft").count();
                        if touched > 0 {
                            ui.label("");
                            ui.checkbox(&mut dialog.day_replace, trf("Change the {} posts edited on the site or already public", &[&touched]))
                                .on_hover_text(tr("Without this, the job stops before sending anything. Public posts keep their status, and edits made on the site are replaced."));
                            ui.end_row();
                        }
                    }

                    // A post already public stays so; a draft is made public only when asked.
                    if !public {
                        ui.label(RichText::new(tr("Readers")).color(theme::muted()));
                        let may = site.as_ref().is_some_and(|site| site.can_publish);
                        let tick = ui.add_enabled(may, egui::Checkbox::new(&mut dialog.publish, tr("Publish it once saved")));
                        if may {
                            tick.on_hover_text(tr("The post is public at once. You confirm what readers will see first."));
                        } else {
                            tick.on_disabled_hover_text(tr("This account may not publish posts on the site; it stays a draft."));
                            dialog.publish = false;
                        }
                        ui.end_row();
                        if dialog.publish {
                            ui.label("");
                            ui.horizontal(|ui| {
                                let tick = ui.checkbox(&mut dialog.at_time, tr("At a set time"));
                                tick.on_hover_text(tr("WordPress makes it public then. The date and time are the site's own, in the time zone set in its settings."));
                                if dialog.at_time {
                                    if dialog.date.is_empty() {
                                        let tomorrow = chrono::Local::now() + chrono::Duration::days(1);
                                        dialog.date = tomorrow.format("%Y-%m-%d").to_string();
                                        dialog.time = "09:00".into();
                                    }
                                    ui.add(egui::TextEdit::singleline(&mut dialog.date).hint_text("2026-11-01").desired_width(90.0));
                                    ui.add(egui::TextEdit::singleline(&mut dialog.time).hint_text("09:00").desired_width(60.0));
                                }
                            });
                            ui.end_row();
                            if dialog.at_time && schedule_of(&dialog.date, &dialog.time).is_none() {
                                ui.label("");
                                ui.label(RichText::new(tr("Type the date as 2026-11-01 and the time as 09:00.")).color(theme::DANGER).size(12.0));
                                ui.end_row();
                            }
                        }
                    }

                    ui.label(RichText::new(tr("Categories and tags")).color(theme::muted()));
                    ui.horizontal(|ui| {
                        let names: Vec<String> = dialog.terms.categories.iter().map(|(_, name)| name.clone()).chain(dialog.terms.tags.iter().map(|(_, name)| format!("#{name}"))).collect();
                        let none = if earlier.is_some() { tr("None chosen: the post keeps the categories and tags it has on the site.") } else { tr("None chosen: the post goes in the site's default category.") };
                        if names.is_empty() {
                            ui.label(RichText::new(tr("Not chosen")).color(theme::muted())).on_hover_text(none);
                        } else {
                            ui.add(egui::Label::new(names.join(", ")).truncate()).on_hover_text(tr("Chosen ones replace the post's categories or tags on the site."));
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
                            let kept = site.as_ref().map_or(0, |site| self.db.publication_media(story_id, site.id).len());
                            // A post saved before its media was recorded says so without a count.
                            if kept > 0 {
                                note(ui, &trf("The {} photos the post used at the larger size stay on the site. The smaller size applies to new uploads only.", &[&kept]));
                            } else {
                                note(ui, tr("Photos uploaded before at the larger size stay on the site. The smaller size applies to new uploads only."));
                            }
                        }
                        if !previews.is_empty() {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for (texture, w, h) in &previews {
                                    let scale = PREVIEW_EDGE / (*w.max(h) as f32).max(1.0);
                                    ui.image((texture.id(), egui::vec2(*w as f32 * scale, *h as f32 * scale)));
                                }
                                let more = dialog.previews.len().saturating_sub(PREVIEW_SHOWN);
                                if more > 0 {
                                    ui.label(RichText::new(format!("+{more}")).color(theme::muted()));
                                }
                            });
                        }
                        if dialog.transparent > 0 {
                            note(ui, &trf("{} photos have transparent areas; they are shown on white, as JPEG keeps no transparency.", &[&dialog.transparent]));
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
                if check.low_res > 0 {
                    ui.label(RichText::new(trf("{} photos are small and may look soft across the page.", &[&check.low_res])).color(theme::muted()));
                }

                // What saving changes on the site, folded away: read only, nothing is sent.
                if let Some(site) = &site {
                    egui::CollapsingHeader::new(RichText::new(tr("Summary")).color(theme::muted())).id_salt("publish-summary").show(ui, |ui| {
                        let update = earlier.is_some() && (dialog.update != Some(false));
                        let title = if dialog.title.is_empty() { tr("(no title)").to_string() } else { dialog.title.clone() };
                        let mut lines = Vec::new();
                        if dialog.by_day {
                            lines.push(trf("An index post titled “{}” and {} Day posts titled after their headings, on {}; those made before are changed, the others made.", &[&title, &dialog.days, &site.label]));
                        } else {
                        match earlier.as_ref().filter(|_| update) {
                            Some(publication) => lines.push(trf("Changes post {} on {}, titled “{}”; its address stays as it is.", &[&publication.post_id, &site.label, &title])),
                            None => lines.push(trf("Makes a new post on {}, titled “{}”, at about {} (WordPress adds a number if it is taken).", &[&site.label, &title, &address_of(&site.site_url, &dialog.title)])),
                        }
                        }
                        let schedule = if dialog.at_time { schedule_of(&dialog.date, &dialog.time) } else { None };
                        lines.push(match (dialog.publish && !public, schedule) {
                            (true, Some(at)) => trf("Readers: public from {} (the site's time).", &[&at.format("%Y-%m-%d %H:%M")]),
                            (true, None) => tr("Readers: public once saved.").to_string(),
                            (false, _) if public => tr("Readers: it keeps its status on the site.").to_string(),
                            _ => tr("Readers: none; it is a draft.").to_string(),
                        });
                        lines.push(trf("Photos: {}, uploaded and named like {}, or reused when already on the site.", &[&dialog.check.photos, &dialog.sample_name]));
                        if dialog.routes > 0 {
                            lines.push(trf("Route maps: {}, uploaded as pictures.", &[&dialog.routes]));
                        }
                        let names = |list: &[(i64, String)]| list.iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>().join(", ");
                        let kept = if update { tr("kept as on the site") } else { tr("the site's default") };
                        lines.push(trf("Categories: {}.", &[&if dialog.terms.categories.is_empty() { kept.to_string() } else { names(&dialog.terms.categories) }]));
                        let kept = if update { tr("kept as on the site") } else { tr("none") };
                        lines.push(trf("Tags: {}.", &[&if dialog.terms.tags.is_empty() { kept.to_string() } else { names(&dialog.terms.tags) }]));
                        lines.push(if dialog.excerpt { tr("Excerpt: the subtitle.") } else { tr("Excerpt: none.") }.to_string());
                        lines.push(if dialog.cover { tr("Featured image: the cover.") } else { tr("Featured image: none.") }.to_string());
                        for line in lines {
                            note(ui, &format!("• {line}"));
                        }
                    });
                }
                // The last tries on this site, folded away.
                let history = site.as_ref().map(|site| self.db.publish_jobs(story_id, site.id)).unwrap_or_default();
                if !history.is_empty() {
                    egui::CollapsingHeader::new(RichText::new(tr("History")).color(theme::muted())).id_salt("publish-history").show(ui, |ui| {
                        for job in history.iter().take(HISTORY_SHOWN) {
                            note(ui, &format!("{} · {} · {}", local_time(job.ended_at), outcome_name(&job.outcome), trf("{} photos uploaded, {} already there", &[&job.uploaded, &job.reused])));
                        }
                    });
                }
                // The button is the approval: it names what is sent, and this says where.
                if let Some(site) = &site {
                    note(ui, &trf("Photos are uploaded to {} first; they can be opened by their address even while the post is a draft.", &[&site.label]));
                }
                if !dialog.error.is_empty() {
                    ui.label(RichText::new(&dialog.error).color(theme::DANGER));
                }
                ui.horizontal(|ui| {
                    let ready = site.as_ref().is_some_and(usable) && !blocked && (dialog.by_day || ((!(published || changed) || dialog.update.is_some()) && (found.is_none() || dialog.adopt.is_some()))) && !(dialog.publish && dialog.at_time && schedule_of(&dialog.date, &dialog.time).is_none());
                    let photos = dialog.check.photos;
                    let update = (published || changed) && dialog.update == Some(true);
                    let publish = dialog.publish && !public;
                    let action = match (photos, update, publish) {
                        _ if dialog.by_day && publish => trf("Upload {} photos and publish {} posts…", &[&photos, &(dialog.days + 1)]),
                        _ if dialog.by_day => trf("Upload {} photos and save {} posts", &[&photos, &(dialog.days + 1)]),
                        (0, _, true) => tr("Publish…").to_string(),
                        (_, _, true) => trf("Upload {} photos and publish…", &[&photos]),
                        (0, true, _) => tr("Update post").to_string(),
                        (0, false, _) => tr("Save draft").to_string(),
                        (_, true, _) => trf("Upload {} photos and update the post", &[&photos]),
                        (_, false, _) => trf("Upload {} photos and save the draft", &[&photos]),
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
        // Making it public names what readers will see, and waits for a yes.
        let day_public = dialog.by_day && site.as_ref().is_some_and(|site| self.db.day_posts(story_id, site.id).iter().any(|(_, post)| post.remote_status != "draft"));
        let public = if dialog.by_day { day_public } else { earlier.as_ref().is_some_and(|publication| publication.remote_status != "draft" || publication.changed_on_site) };
        let publish = dialog.publish && !public;
        let schedule = if publish && dialog.at_time { schedule_of(&dialog.date, &dialog.time) } else { None };
        let by_day = dialog.by_day && dialog.days > 0;
        let day_replace = by_day && dialog.day_replace;
        if start && publish && !dialog.confirming {
            dialog.confirming = true;
            start = false;
        }
        if dialog.confirming {
            let mut back = false;
            let label = site.as_ref().map(|site| site.label.clone()).unwrap_or_default();
            egui::Modal::new(egui::Id::unique("publish-confirm")).frame(about::dialog_frame()).show(ctx, |ui| {
                ui.set_width(400.0);
                about::title_bar(ui, &self.logo, &trf("Publish on {}?", &[&label]), &mut back);
                egui::Frame::new().inner_margin(Margin::symmetric(20, 14)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    match schedule {
                        Some(at) => ui.label(trf("Anyone can read the post from {} (the site's time):", &[&at.format("%Y-%m-%d %H:%M")])),
                        None => ui.label(tr("Anyone can read the post as soon as it is saved:")),
                    };
                    let title = if dialog.title.is_empty() { tr("(no title)").to_string() } else { dialog.title.clone() };
                    for line in [
                        trf("The title “{}”, the text, captions and photo descriptions", &[&title]),
                        if dialog.exif {
                            trf("{} photos, with their camera details but never where they were taken", &[&dialog.check.photos])
                        } else {
                            trf("{} photos, without camera details or where they were taken", &[&dialog.check.photos])
                        },
                        if dialog.routes > 0 { trf("{} route maps, which show where the trip went", &[&dialog.routes]) } else { tr("No route maps").to_string() },
                    ] {
                        ui.label(format!("• {line}"));
                    }
                    note(ui, tr("Picasso never takes a post down; that is done in WordPress."));
                    ui.horizontal(|ui| {
                        start = ui.add(theme::primary_button(tr("Publish"))).clicked();
                        back |= ui.button(tr("Back")).clicked();
                    });
                });
            });
            if back || start {
                dialog.confirming = false;
            }
        }
        if settings {
            self.modal = Modal::None;
            self.open_settings(ctx);
            return;
        }
        if let Some((site, (taxonomy, name))) = site.as_ref().zip(dialog.site_terms.make.take()) {
            match connection(&self.vault, site) {
                Ok((api, credentials, policy)) => {
                    let (sender, receiver) = crossbeam_channel::bounded(1);
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let network = Network::new();
                        let made = Client::new(&network, policy).create_term(&api, &credentials, taxonomy, &name);
                        let _ = sender.send(made.map(|term| (taxonomy, term)));
                        ctx.request_repaint();
                    });
                    dialog.site_terms.error.clear();
                    dialog.site_terms.making = Some(receiver);
                }
                Err(err) => dialog.site_terms.error = err,
            }
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
        let publish = publish && start;
        // A new draft beside a published post replaces the record of it here, never the post.
        let update = dialog.update == Some(true);
        let earlier = earlier.filter(|publication| (publication.remote_status == "draft" && !publication.changed_on_site) || update);
        // Taking up the post found on the site: changed in place, keeping its status.
        let adopted = found.filter(|_| dialog.adopt == Some(true));
        let allow_new = found.is_none() || dialog.adopt == Some(false);
        let (earlier, update) = match adopted {
            Some(post_id) => (Some(Publication { story_id, site_id: site.id, post_id, remote_status: "publish".into(), size: size.code().into(), revision: 0, synced_at: 0, fingerprint: String::new(), changed_on_site: false }), true),
            None => (earlier, update),
        };
        match self.start_publishing(ctx, &site, size, exif, terms, earlier, if by_day { day_replace } else { update }, publish, schedule, allow_new, by_day) {
            Ok(()) => {
                self.modal = Modal::None;
                self.publish_found = None;
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
    fn start_publishing(&mut self, ctx: &egui::Context, site: &Site, size: UploadSize, exif: bool, terms: PostTerms, earlier: Option<Publication>, update: bool, publish: bool, schedule: Option<chrono::NaiveDateTime>, allow_new: bool, by_day: bool) -> Result<(), String> {
        let (api, credentials, policy) = connection(&self.vault, site)?;
        if !self.save_story() {
            return Err(tr("The story couldn't be saved").into());
        }
        let Some(page) = self.story_job() else { return Err(tr("The story couldn't be saved").into()) };
        self.db.save_post_terms(page.story.id, site.id, &terms)?;
        let pending_since = if earlier.is_none() { self.db.pending_draft(page.story.id, site.id) } else { None };
        let days = by_day.then(|| {
            let known = self.db.day_posts(page.story.id, site.id);
            known.into_iter().map(|(key, publication)| (key, publish::DayPost { post_id: publication.post_id, status: publication.remote_status, fingerprint: publication.fingerprint, changed_on_site: publication.changed_on_site })).collect()
        });
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
            overwrite: update && (by_day || earlier.as_ref().is_some_and(|publication| publication.changed_on_site)),
            pending_since,
            publish,
            schedule,
            days,
            allow_new,
            map: Some(page.map),
            profiles: page.profiles,
            update_published: update && (by_day || earlier.as_ref().is_some_and(|publication| publication.remote_status != "draft")),
            terms,
        };
        let progress = self.start_story_job();
        let (story_id, site_id, revision, label) = (job.story.id, site.id, job.story.revision, site.label.clone());
        let events = self.events_tx.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("picasso-publish".into()).spawn(move || {
            let started_at = unix_now();
            let network = Network::new();
            let run = if job.days.is_some() { publish::run_days } else { publish::run };
            let outcome = run(&network, &job, &progress, &mut |event| {
                let _ = events.send(UiEvent::PublishProgress(site_id, event));
                ctx.request_repaint();
            });
            let finished = Finished { story_id, site_id, site: label, size, revision, started_at, outcome };
            let _ = events.send(UiEvent::StoryPublished(Box::new(finished)));
            ctx.request_repaint();
        });
        Ok(())
    }

    /// Keeps what the job learned the moment it did, so a retry reuses it.
    pub(super) fn on_publish_progress(&mut self, site_id: i64, event: Event) {
        let kept = match event {
            Event::Uploaded { hash, size, media_id, url, details } => self.db.record_upload(site_id, &hash, size.code(), media_id, &url, &details),
            Event::Gone { hash } => self.db.forget_upload(site_id, &hash),
            Event::Creating { story_id, since } => self.db.set_pending_draft(story_id, site_id, since),
            Event::Part { story_id, revision, key, post } => {
                let publication = Publication {
                    story_id,
                    site_id,
                    post_id: post.post_id,
                    remote_status: post.status,
                    size: String::new(),
                    revision,
                    synced_at: unix_now(),
                    fingerprint: post.fingerprint,
                    changed_on_site: post.changed_on_site,
                };
                self.db.save_day_post(&key, &publication)
            }
            Event::Prepared { photos, bytes } => {
                self.activity = trf("{} photos ready, {} in all; sending those not on the site yet…", &[&photos, &format_bytes(bytes)]);
                Ok(())
            }
        };
        if let Err(err) = kept {
            self.activity = trf("Couldn't record an upload: {}", &[&err]);
        }
    }

    pub(super) fn on_story_published(&mut self, finished: Finished) {
        self.end_story_job(finished.story_id);
        let Finished { story_id, site_id, site, size, revision, started_at, outcome } = finished;
        let (uploaded, reused) = publish::counts(&outcome);
        let logged = crate::db::PublishJob {
            story_id,
            site_id,
            started_at,
            ended_at: unix_now(),
            outcome: publish::outcome_code(&outcome),
            post_id: outcome.as_ref().ok().map(|saved| saved.post_id),
            uploaded,
            reused,
            size: size.code().into(),
            revision,
        };
        let _ = self.db.record_publish_job(&logged);
        self.activity = match outcome {
            // Each post was recorded as it was saved; the index opens.
            Ok(saved) if saved.days > 0 => {
                let _ = open::that(&saved.edit_link);
                if saved.published && saved.status == "future" {
                    trf("Scheduled on {}: the index and {} Day posts go public at the time chosen. {} photos uploaded, {} already there.", &[&site, &saved.days, &saved.uploaded, &saved.reused])
                } else if saved.published {
                    trf("Published on {}: the index and {} Day posts. {} photos uploaded, {} already there.", &[&site, &saved.days, &saved.uploaded, &saved.reused])
                } else {
                    trf("Saved on {}: the index and {} Day posts. {} photos uploaded, {} already there.", &[&site, &saved.days, &saved.uploaded, &saved.reused])
                }
            }
            Ok(saved) => {
                let _ = self.db.clear_pending_draft(story_id, site_id);
                let _ = self.db.save_publication_media(story_id, site_id, &saved.media);
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
                    if saved.published && saved.status == "future" {
                        trf("Scheduled on {}: it goes public at the time chosen. {} photos uploaded, {} already there.", &[&site, &saved.uploaded, &saved.reused])
                    } else if saved.published && saved.scheduled {
                        trf("{} made the post public at once rather than at the time chosen.", &[&site])
                    } else if saved.published {
                        trf("Published on {}: {} photos uploaded, {} already there.", &[&site, &saved.uploaded, &saved.reused])
                    } else if saved.status == "draft" {
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
                    trf("The post on {} isn't a draft any more ({}). Nothing was changed; publish again to update it or to save a new draft.", &[&site, &status_name(&status)])
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
            Err(Stopped::Uncertain { drafts }) => {
                trf("A draft made on {} may not have been confirmed, and {} drafts there could be it. Nothing more was made; delete the extra drafts on the site, then publish again.", &[&site, &drafts])
            }
            Err(Stopped::TitleOnSite { post_id }) => {
                // Kept for this session, so the dialog asks whether to take it up.
                self.publish_found = Some((story_id, site_id, post_id));
                trf("{} has a post with this story's title that Picasso has no record of. Nothing was sent; publish again to update it or to make a new post.", &[&site])
            }
            Err(Stopped::TooSoon) => trf("The time chosen has passed, or is less than {} minutes away, on {}'s clock. Nothing was sent; choose a later time.", &[&publish::SCHEDULE_AHEAD, &site]),
            Err(Stopped::PostGone) => {
                let _ = self.db.forget_publication(story_id, site_id);
                trf("The earlier draft is gone from {}. Save the draft again to make a new one.", &[&site])
            }
        };
    }
}

/// The dialog's width in points: room for the label column and the photo preview.
const DIALOG_WIDTH: f32 = 500.0;

/// Photos shown as the dialog's preview, and their longest side in points.
const PREVIEW_SHOWN: usize = 8;
const PREVIEW_EDGE: f32 = 40.0;

/// Tries listed under History in the dialog.
const HISTORY_SHOWN: usize = 5;

/// How a job ended (`publish::outcome_code`), in words.
fn outcome_name(code: &str) -> String {
    match code.split(':').next().unwrap_or_default() {
        "published" => tr("Published").into(),
        "saved" => tr("Saved as a draft").into(),
        "updated" => tr("Updated").into(),
        "cancelled" => tr("Cancelled").into(),
        "site" => tr("The site stopped it").into(),
        "photo unavailable" => tr("A photo was unavailable").into(),
        "unsupported block" => tr("A block can't be published").into(),
        "not a draft" => tr("The post wasn't a draft").into(),
        "post gone" => tr("The post was gone").into(),
        "changed on site" => tr("Edited on the site").into(),
        "uncertain" => tr("More than one draft could be it").into(),
        "title on site" => tr("A post with its title was on the site").into(),
        "scheduled" => tr("Scheduled").into(),
        "too soon" => tr("The time chosen had passed").into(),
        other => other.to_string(),
    }
}

/// About the address WordPress gives a new post titled `title`: lowercase,
/// accents dropped, and runs of anything but letters and digits made one
/// dash. The site may differ, as when the address is taken.
fn slug_of(title: &str) -> String {
    let plain: String = title.chars().map(|ch| match ch {
        'à'..='å' | 'À'..='Å' => 'a',
        'ç' | 'Ç' => 'c',
        'è'..='ë' | 'È'..='Ë' => 'e',
        'ì'..='ï' | 'Ì'..='Ï' => 'i',
        'ñ' | 'Ñ' => 'n',
        'ò'..='ö' | 'Ò'..='Ö' => 'o',
        'ù'..='ü' | 'Ù'..='Ü' => 'u',
        other => other,
    }).collect();
    let mut out = String::new();
    for ch in plain.to_lowercase().chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// About where a new post titled `title` will be read on `site`; one
/// with no letters or digits in its title gets a number from the site.
fn address_of(site: &str, title: &str) -> String {
    let site = site.trim_end_matches('/');
    match slug_of(title) {
        slug if slug.is_empty() => format!("{site}/?p=…"),
        slug => format!("{site}/{slug}/"),
    }
}

/// The date and time typed for a scheduled post, such as "2026-11-01" and "9:00".
fn schedule_of(date: &str, time: &str) -> Option<chrono::NaiveDateTime> {
    let date = chrono::NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").ok()?;
    let time = chrono::NaiveTime::parse_from_str(time.trim(), "%H:%M").ok()?;
    Some(date.and_time(time))
}

/// A post status as WordPress names it, in words; one it doesn't know stays as given.
fn status_name(status: &str) -> String {
    match status {
        "publish" => tr("published").into(),
        "future" => tr("scheduled").into(),
        "pending" => tr("pending review").into(),
        "private" => tr("private").into(),
        "draft" => tr("draft").into(),
        other => other.to_string(),
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
        note(ui, tr("Chosen ones replace the post's categories or tags on the site."));
    }
    if !site.error.is_empty() {
        ui.label(RichText::new(&site.error).color(theme::DANGER));
    }
    if site.reading.is_some() || site.making.is_some() {
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
    // A name the site doesn't have yet can be added, as a category, a tag, or both.
    let name = site.filter.trim().to_string();
    let missing = |list: &[Term]| !name.is_empty() && !list.iter().any(|term| term.name.to_lowercase() == wanted);
    let (new_category, new_tag) = (missing(categories), missing(tags));
    if new_category || new_tag {
        ui.horizontal_wrapped(|ui| {
            if new_category && ui.small_button(trf("Add category \"{}\"", &[&name])).on_hover_text(tr("Makes it on the site and chooses it")).clicked() {
                site.make = Some((Taxonomy::Categories, name.clone()));
            }
            if new_tag && ui.small_button(trf("Add tag \"{}\"", &[&name])).on_hover_text(tr("Makes it on the site and chooses it")).clicked() {
                site.make = Some((Taxonomy::Tags, name.clone()));
            }
        });
    }
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
    fn a_new_posts_address_is_guessed_from_its_title() {
        assert_eq!(slug_of("Spain – 2025-07-01"), "spain-2025-07-01");
        assert_eq!(slug_of("Café in Sevilla!"), "cafe-in-sevilla");
        assert_eq!(slug_of("京都の秋"), "京都の秋");
        assert_eq!(address_of("http://localhost:10003/", " — "), "http://localhost:10003/?p=…");
        assert_eq!(address_of("https://example.com/blog", "Spain"), "https://example.com/blog/spain/");
    }

    #[test]
    fn a_typed_date_and_time_are_read_or_refused() {
        assert_eq!(schedule_of(" 2026-11-01 ", "9:05").map(|at| at.to_string()), Some("2026-11-01 09:05:00".into()));
        assert_eq!(schedule_of("2026-02-30", "09:00"), None);
        assert_eq!(schedule_of("2026-11-01", "25:00"), None);
    }

    #[test]
    fn categories_list_each_child_after_its_parent() {
        let term = |id, name: &str, parent| Term { id, name: name.into(), parent };
        let categories = [term(2, "Europe", 0), term(6, "Granada", 5), term(5, "Spain", 2), term(1, "Uncategorized", 0), term(9, "Orphan", 77), term(4, "Loop", 4)];
        let listed: Vec<(usize, &str)> = by_parent(&categories).into_iter().map(|(depth, term)| (depth, term.name.as_str())).collect();
        assert_eq!(listed, [(0, "Europe"), (1, "Spain"), (2, "Granada"), (0, "Uncategorized"), (0, "Orphan"), (0, "Loop")]);
    }
}
