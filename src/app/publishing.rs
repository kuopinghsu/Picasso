//! The Publishing page of Settings: the WordPress sites Stories can be
//! published to. Connecting a site finds its API and checks the account on
//! a thread; the Application Password goes to the system's credential
//! store and only a reference to it to the library (`docs/TRAVEL.md`).

use crossbeam_channel::Receiver;
use eframe::egui::{self, Margin, RichText};

use crate::db::Site;
use crate::i18n::{tr, trf};
use crate::keychain::{self, Kept};
use crate::types::unix_now;
use crate::wordpress::{Account, Api, Client, Credentials, Failure, Network, Policy, SiteUrl};

use super::settings::{heading, local_time, note};
use super::{theme, PicassoApp};

/// What checking a site finds: its API and the account, or why not.
type Checked = Result<(Api, Account), Failure>;

/// What the page keeps between frames.
#[derive(Default)]
pub(super) struct PublishingState {
    /// The site being connected or changed.
    form: Option<SiteForm>,
    /// A check running on a thread, for the site with this id (0 for a new one).
    checking: Option<(i64, Receiver<Checked>)>,
    /// What the latest action did, or why it failed.
    note: String,
    error: String,
    /// A site waiting for Disconnect to be confirmed.
    disconnect: Option<i64>,
}

/// The fields of Connect site, or of Edit for a connected one.
#[derive(Clone, Default)]
struct SiteForm {
    /// 0 for a new site.
    id: i64,
    label: String,
    address: String,
    user: String,
    password: String,
    /// Plain HTTP to a site on this computer, for development.
    development: bool,
}

impl PicassoApp {
    pub(super) fn publishing_page(&mut self, ui: &mut egui::Ui) {
        self.publishing_results(ui.ctx());
        heading(ui, tr("WordPress sites"));
        note(
            ui,
            tr("Publish Stories to your own WordPress site. Picasso signs in with an Application Password, which you make in WordPress under Users → Profile → Application Passwords. It is kept in this computer's password store, never in the library or its backups."),
        );
        let sites = self.db.sites();
        egui::Frame::new()
            .fill(theme::tray())
            .stroke(egui::Stroke::new(1.0, theme::hairline()))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().id_salt("publishing-sites").max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    if sites.is_empty() {
                        ui.label(RichText::new(tr("No sites yet")).color(theme::muted()));
                    }
                    for site in &sites {
                        self.site_row(ui, site);
                    }
                });
            });
        let busy = self.settings.publishing.checking.is_some();
        if self.settings.publishing.form.is_none() && ui.add_enabled(!busy, egui::Button::new(tr("Connect site…"))).clicked() {
            self.settings.publishing.form = Some(SiteForm::default());
            self.settings.publishing.error.clear();
            self.settings.publishing.note.clear();
        }
        if self.settings.publishing.form.is_some() {
            self.site_form(ui);
        }
        let state = &self.settings.publishing;
        if state.checking.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0));
                ui.label(RichText::new(tr("Checking…")).color(theme::muted()));
            });
        }
        if !state.error.is_empty() {
            ui.add(egui::Label::new(RichText::new(&state.error).color(theme::DANGER)).wrap());
        } else if !state.note.is_empty() {
            ui.add(egui::Label::new(RichText::new(&state.note).color(theme::muted())).wrap());
        }
    }

    fn site_row(&mut self, ui: &mut egui::Ui, site: &Site) {
        let busy = self.settings.publishing.checking.is_some();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(RichText::new(&site.label).strong()).truncate());
                let who = if site.account.is_empty() { site.user.clone() } else { format!("{} ({})", site.account, site.user) };
                ui.add(egui::Label::new(RichText::new(format!("{} · {who}", site.site_url)).size(12.0).color(theme::muted())).truncate());
                ui.horizontal(|ui| {
                    for (can, label) in [(site.can_upload, tr("Upload")), (site.can_edit, tr("Edit posts")), (site.can_publish, tr("Publish"))] {
                        // A mark as well as a color, so it reads without color too.
                        let (mark, color) = if can { ("✓", theme::text()) } else { ("✗", theme::muted()) };
                        ui.label(RichText::new(format!("{mark} {label}")).size(12.0).color(color));
                    }
                    let checked = site.checked_at.map_or_else(|| tr("Never checked").to_string(), |at| trf("Checked {}", &[&local_time(at)]));
                    ui.label(RichText::new(checked).size(12.0).color(theme::muted()));
                });
            });
        });
        ui.horizontal(|ui| {
            if self.settings.publishing.disconnect == Some(site.id) {
                ui.add(
                    egui::Label::new(RichText::new(trf("Disconnect {}? Picasso forgets the site and its password. To stop the password working, revoke it in WordPress under Users → Profile → Application Passwords.", &[&site.label])).size(12.5))
                        .wrap(),
                );
                if ui.add(theme::danger_button(tr("Disconnect"))).clicked() {
                    self.disconnect_site(site);
                }
                if ui.button(tr("Cancel")).clicked() {
                    self.settings.publishing.disconnect = None;
                }
                return;
            }
            if ui.add_enabled(!busy, egui::Button::new(tr("Check"))).on_hover_text(tr("Sign in again and read what the account may do. Nothing is changed on the site.")).clicked() {
                self.check_site(ui.ctx(), site);
            }
            if ui.add_enabled(!busy, egui::Button::new(tr("Edit…"))).clicked() {
                self.settings.publishing.form = Some(SiteForm {
                    id: site.id,
                    label: site.label.clone(),
                    address: site.site_url.clone(),
                    user: site.user.clone(),
                    password: String::new(),
                    development: SiteUrl::parse(&site.site_url).is_ok_and(|url| url.scheme == "http"),
                });
                self.settings.publishing.error.clear();
                self.settings.publishing.note.clear();
            }
            if ui.add_enabled(!busy, egui::Button::new(tr("Disconnect"))).clicked() {
                self.settings.publishing.disconnect = Some(site.id);
            }
        });
        ui.separator();
    }

    fn site_form(&mut self, ui: &mut egui::Ui) {
        let busy = self.settings.publishing.checking.is_some();
        let Some(form) = self.settings.publishing.form.as_mut() else { return };
        let mut submit = false;
        let mut cancel = false;
        egui::Grid::new("site-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(tr("Site address"));
            let address = ui.add(egui::TextEdit::singleline(&mut form.address).hint_text("example.com/blog").desired_width(280.0));
            if form.id == 0 && ui.memory(|memory| memory.focused().is_none()) {
                address.request_focus();
            }
            ui.end_row();
            ui.label(tr("User name"));
            ui.add(egui::TextEdit::singleline(&mut form.user).desired_width(280.0));
            ui.end_row();
            ui.label(tr("Application Password"));
            let password = ui.add(egui::TextEdit::singleline(&mut form.password).password(true).desired_width(280.0));
            submit |= password.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            ui.end_row();
            ui.label(tr("Name"));
            ui.add(egui::TextEdit::singleline(&mut form.label).hint_text(tr("Travel blog")).desired_width(280.0));
            ui.end_row();
        });
        if form.id != 0 {
            note(ui, tr("To change the password, make a new Application Password in WordPress, enter it here, and revoke the old one there."));
        }
        let local = SiteUrl::parse(&form.address).is_ok_and(|url| url.is_development_host());
        if local {
            ui.checkbox(&mut form.development, tr("Development site on this computer: allow plain HTTP"));
        } else {
            form.development = false;
        }
        let ready = !form.address.trim().is_empty() && !form.user.trim().is_empty() && !form.password.is_empty();
        ui.horizontal(|ui| {
            let label = if form.id == 0 { tr("Connect") } else { tr("Save") };
            submit |= ui.add_enabled(ready && !busy, theme::primary_button(label)).clicked();
            cancel = ui.button(tr("Cancel")).clicked();
        });
        if cancel {
            self.settings.publishing.form = None;
            self.settings.publishing.error.clear();
        } else if submit && ready && !busy {
            let form = form.clone();
            self.start_check(ui.ctx(), form.id, &form.address, &form.user, &form.password, form.development);
        }
    }

    /// Checks a connected site again with the password in the credential store.
    fn check_site(&mut self, ctx: &egui::Context, site: &Site) {
        let state = &mut self.settings.publishing;
        state.error.clear();
        state.note.clear();
        match self.vault.load(&site.secret_ref) {
            Ok(Some(password)) => {
                let development = SiteUrl::parse(&site.site_url).is_ok_and(|url| url.scheme == "http");
                self.start_check(ctx, site.id, &site.site_url, &site.user, &password, development);
            }
            Ok(None) => {
                state.error = tr("Picasso doesn't have this site's password any more. Use Edit… to enter it again.").to_string();
            }
            Err(err) => state.error = store_text(&err),
        }
    }

    fn start_check(&mut self, ctx: &egui::Context, id: i64, address: &str, user: &str, password: &str, development: bool) {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let (address, credentials, ctx) = (address.trim().to_string(), Credentials { user: user.trim().to_string(), password: password.to_string() }, ctx.clone());
        std::thread::spawn(move || {
            let network = Network::new();
            let client = Client::new(&network, Policy { development });
            let result = client.discover(&address).and_then(|api| client.account(&api, &credentials).map(|account| (api, account)));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        let state = &mut self.settings.publishing;
        state.checking = Some((id, receiver));
        state.error.clear();
        state.note.clear();
    }

    /// Takes in a finished check: keeps the password and the site when it
    /// worked, or says why it didn't.
    fn publishing_results(&mut self, ctx: &egui::Context) {
        let Some((id, receiver)) = &self.settings.publishing.checking else { return };
        let id = *id;
        let Ok(result) = receiver.try_recv() else {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        };
        self.settings.publishing.checking = None;
        let (api, account) = match result {
            Ok(found) => found,
            Err(failure) => {
                self.settings.publishing.error = failure_text(&failure);
                return;
            }
        };
        let form = self.settings.publishing.form.clone().filter(|form| form.id == id);
        let existing = (id != 0).then(|| self.db.site(id)).flatten();
        let user = form.as_ref().map_or_else(|| existing.as_ref().map(|site| site.user.clone()).unwrap_or_default(), |form| form.user.trim().to_string());
        let site_url = api.site.to_url();
        let secret_ref = keychain::wordpress_reference(&api.site.origin(), &user);
        let mut kept = None;
        if let Some(form) = &form {
            match self.vault.save(&secret_ref, &form.password) {
                Ok(where_) => kept = Some(where_),
                Err(err) => {
                    self.settings.publishing.error = store_text(&err);
                    return;
                }
            }
        }
        let site = Site {
            id,
            label: form.as_ref().map(|form| form.label.clone()).or_else(|| existing.as_ref().map(|site| site.label.clone())).unwrap_or_default(),
            site_url,
            api_root: api.root.to_url(),
            user,
            secret_ref: secret_ref.clone(),
            account: account.name.clone(),
            can_upload: account.upload_files,
            can_edit: account.edit_posts,
            can_publish: account.publish_posts,
            checked_at: Some(unix_now()),
        };
        match self.db.save_site(&site) {
            Ok(_) => {
                // A site that moved, or an account renamed, leaves its old password behind otherwise.
                if let Some(old) = existing.filter(|old| old.secret_ref != secret_ref) {
                    let _ = self.vault.delete(&old.secret_ref);
                }
                let mut text = trf("Signed in to {} as {}.", &[&api.name, &account.name]);
                if !account.upload_files || !account.edit_posts {
                    text.push(' ');
                    text.push_str(tr("This account can't upload photos or write posts, so Picasso can't publish with it."));
                }
                if kept == Some(Kept::Session) {
                    text.push(' ');
                    text.push_str(tr("No password store is available, so the password is only kept until Picasso quits."));
                }
                self.settings.publishing.note = text;
                self.settings.publishing.form = None;
            }
            Err(err) => self.settings.publishing.error = err,
        }
    }

    fn disconnect_site(&mut self, site: &Site) {
        let state = &mut self.settings.publishing;
        state.disconnect = None;
        if let Err(err) = self.vault.delete(&site.secret_ref) {
            state.error = store_text(&err);
            return;
        }
        match self.db.delete_site(site.id) {
            Ok(()) => state.note = trf("Disconnected {}.", &[&site.label]),
            Err(err) => state.error = err,
        }
    }
}

/// What a person can do about `failure`.
pub(super) fn failure_text(failure: &Failure) -> String {
    match failure {
        Failure::InvalidUrl => tr("That isn't a web address. Enter the site's address, such as example.com/blog.").to_string(),
        Failure::NotHttps => tr("The site has to use HTTPS. Enter its address with https://.").to_string(),
        Failure::PrivateAddress => tr("That address is on a private network. Only a development site on this computer can be reached that way.").to_string(),
        Failure::Unreachable => tr("The site couldn't be reached. Check the address and your internet connection.").to_string(),
        Failure::Tls => tr("The site's security certificate couldn't be verified.").to_string(),
        Failure::Timeout => tr("The site took too long to answer. Try again later.").to_string(),
        Failure::CrossOriginRedirect(origin) => trf("The site sent Picasso on to {}, so the password wasn't sent. Enter the address the site really uses.", &[origin]),
        Failure::TooManyRedirects => tr("The site keeps redirecting. Check its address.").to_string(),
        Failure::NotWordPress => tr("No WordPress REST API was found at that address. Check the address, including any folder the site is in.").to_string(),
        Failure::ApiBlocked => tr("The site's REST API is turned off or blocked, often by a security plugin. Ask the site's administrator to allow it.").to_string(),
        Failure::Unauthorized => tr("The user name or Application Password was refused. Check both, or make a new Application Password.").to_string(),
        Failure::CredentialsIgnored => tr("The site ignored the Application Password. WordPress accepts them only over HTTPS, or on a site whose environment type is \"local\"; it also ignores them until one has been made on the site, and some servers drop the sign-in. For a development site, add define( 'WP_ENVIRONMENT_TYPE', 'local' ); to its wp-config.php.").to_string(),
        Failure::Forbidden => tr("This account isn't allowed to do that on the site.").to_string(),
        Failure::TooLarge => tr("The site refused a photo as larger than it accepts. Choose Low resolution, or raise the site's upload limit.").to_string(),
        Failure::RateLimited(Some(seconds)) => trf("The site asked Picasso to slow down. Try again in {} seconds.", &[seconds]),
        Failure::RateLimited(None) => tr("The site asked Picasso to slow down. Try again later.").to_string(),
        Failure::Status(status) => trf("The site answered with an error ({}). Try again later.", &[status]),
        Failure::BadReply => tr("The site's answer couldn't be read.").to_string(),
    }
}

fn store_text(err: &keychain::StoreError) -> String {
    match err {
        keychain::StoreError::Unavailable => tr("This computer's password store couldn't be used.").to_string(),
        keychain::StoreError::Failed(why) => trf("This computer's password store refused: {}", &[why]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_failure_has_its_own_advice() {
        let failures = [
            Failure::InvalidUrl,
            Failure::NotHttps,
            Failure::PrivateAddress,
            Failure::Unreachable,
            Failure::Tls,
            Failure::Timeout,
            Failure::CrossOriginRedirect("https://evil.example".into()),
            Failure::TooManyRedirects,
            Failure::NotWordPress,
            Failure::ApiBlocked,
            Failure::Unauthorized,
            Failure::CredentialsIgnored,
            Failure::Forbidden,
            Failure::TooLarge,
            Failure::RateLimited(Some(30)),
            Failure::RateLimited(None),
            Failure::Status(500),
            Failure::BadReply,
        ];
        let texts: std::collections::HashSet<String> = failures.iter().map(failure_text).collect();
        assert_eq!(texts.len(), failures.len());
        assert!(failure_text(&Failure::RateLimited(Some(30))).contains("30"));
    }
}
