#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use crate::i18n::{tr, trf, Language};
mod api;
mod cache;
mod compact;
mod data;
mod demo;
mod i18n;
mod shine;
mod theme;
mod vault;
mod views;

use chrono::{Duration, NaiveDate};
use data::{Message, Payload, Section};
use eframe::egui::{self, Color32, RichText, Vec2};
use std::{
    collections::HashMap,
    sync::{mpsc, Arc},
};
use theme::{Accent, GREEN, MUTED, RED, TEXT};
use zeroize::Zeroize;

enum Event {
    Login {
        epoch: u64,
        result: Result<(Arc<api::Session>, Option<String>), String>,
    },
    Data {
        epoch: u64,
        section: Section,
        revision: u64,
        result: Result<Payload, String>,
    },
    Detail {
        epoch: u64,
        id: String,
        result: Result<Message, String>,
    },
    Logout {
        epoch: u64,
        result: Result<(), String>,
    },
}
#[derive(Default)]
struct LoadState {
    loading: bool,
    error: Option<String>,
    updated: Option<String>,
    revision: u64,
}
struct App {
    login: String,
    password: String,
    remember: bool,
    session: Option<Arc<api::Session>>,
    section: Section,
    accent: Accent,
    language: Language,
    status: String,
    auth_busy: bool,
    epoch: u64,
    data: HashMap<Section, Payload>,
    loads: HashMap<Section, LoadState>,
    view: views::ViewState,
    week: NaiveDate,
    page: usize,
    selected: Option<String>,
    detail: Option<Message>,
    detail_error: Option<String>,
    detail_loading: bool,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    demo: bool,
    archive: Option<cache::Archive>,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let language = Language::load();
        language.activate();
        theme::fonts(&cc.egui_ctx);
        let accent = Accent::load();
        theme::apply(&cc.egui_ctx, accent);
        let (tx, rx) = mpsc::channel();
        let preview = std::env::args().any(|a| a == "--demo");
        let mut app = Self {
            login: String::new(),
            password: String::new(),
            remember: false,
            session: None,
            section: Section::Account,
            accent,
            language,
            status: tr!(
                "Zaloguj się danymi konta Synergia otrzymanymi w szkole.",
                "Sign in with the Synergia account provided by your school."
            )
            .into(),
            auth_busy: false,
            epoch: 0,
            data: HashMap::new(),
            loads: HashMap::new(),
            view: views::ViewState::default(),
            week: data::monday(data::today()),
            page: 1,
            selected: None,
            detail: None,
            detail_error: None,
            detail_loading: false,
            tx,
            rx,
            demo: preview,
            archive: None,
        };
        if preview {
            app.login = tr!("Konto pokazowe", "Demo account").into();
            app.section = Section::Timetable;
            if std::env::args().any(|a| a == "--demo-grades") {
                app.section = Section::Grades;
            }
            app.data = demo::payloads(app.week);
            app.status = tr!(
                "PODGLĄD · fikcyjne dane przykładowe · bez połączenia z Librusem",
                "PREVIEW · fictional sample data · not connected to Librus"
            )
            .into();
        } else {
            match vault::load() {
                Ok(Some(mut credentials)) => {
                    app.login = credentials.login.clone();
                    app.bind_cache(&credentials.login);
                    app.remember = true;
                    match api::Session::deferred(&credentials.login, &credentials.password) {
                        Ok(session) => {
                            app.session = Some(Arc::new(session));
                            app.status = if app.data.is_empty() {
                                tr!(
                                    "Konto gotowe. Kliknij Odśwież, aby pobrać pierwszy zapis.",
                                    "Account ready. Click Refresh to download your first snapshot."
                                )
                                .into()
                            } else {
                                tr!("Ostatni zapis jest gotowy. Sesja zostanie odnowiona przy następnym pobraniu danych.", "Your last snapshot is ready. The session will renew on your next data request.").into()
                            };
                        }
                        Err(_) => {
                            app.status = tr!(
                                "Nie można przygotować połączenia. Zaloguj się ponownie.",
                                "Could not prepare the connection. Please sign in again."
                            )
                            .into()
                        }
                    }
                    credentials.password.zeroize();
                }
                Ok(None) => {
                    if let Ok(Some(owner)) = cache::last_account() {
                        app.login = owner.clone();
                        app.bind_cache(&owner);
                        app.status = tr!(
                            "Wyświetlasz ostatni zapis offline. Zaloguj się, aby pobrać nowe dane.",
                            "Showing your last offline snapshot. Sign in to download new data."
                        )
                        .into();
                    }
                }
                Err(_) => {
                    app.status = tr!(
                        "Nie udało się odczytać zapisanych danych Windows. Zaloguj się ręcznie.",
                        "Could not read saved Windows credentials. Please sign in manually."
                    )
                    .into()
                }
            }
        }
        app
    }
    fn connected(&self) -> bool {
        self.session.is_some() || self.demo
    }
    fn bind_cache(&mut self, owner: &str) {
        if self.archive.as_ref().is_some_and(|a| a.owner == owner) {
            return;
        }
        self.clear_data();
        let archive = match cache::load(owner) {
            Ok(archive) => archive,
            Err(_) => {
                self.status=tr!("Nie udało się odczytać poprzedniego zapisu. Plik pozostanie zachowany do udanego pobrania nowych danych.", "Could not read the previous snapshot. It will be preserved until new data is downloaded successfully.").into();
                cache::Archive::new(owner.to_owned())
            }
        };
        for (section, snapshot) in &archive.snapshots {
            self.data.insert(*section, snapshot.payload.clone());
            self.loads.insert(
                *section,
                LoadState {
                    updated: Some(snapshot.updated.clone()),
                    ..Default::default()
                },
            );
            if *section == Section::Timetable {
                if let Ok(week) = NaiveDate::parse_from_str(&snapshot.week, "%Y-%m-%d") {
                    self.week = week;
                }
            }
            if *section == Section::Messages {
                self.page = snapshot.page;
            }
        }
        self.section = archive.last_section;
        self.archive = Some(archive);
    }
    fn persist(&mut self, section: Section, payload: &Payload, updated: &str) {
        if self.demo {
            return;
        }
        if let Some(archive) = &mut self.archive {
            archive.snapshots.insert(
                section,
                cache::Snapshot {
                    payload: payload.clone(),
                    updated: updated.into(),
                    week: self.week.to_string(),
                    page: self.page,
                },
            );
            archive.last_section = section;
            if cache::save(archive)
                .and_then(|_| cache::activate(&archive.owner))
                .is_err()
            {
                self.status=tr!("Dane pobrano, ale nie zapisano ich na dysku. Poprzedni plik pozostał zachowany.", "Data downloaded but could not be saved to disk. The previous snapshot was preserved.").into();
            }
        }
    }
    fn restore_snapshot(&mut self, section: Section) {
        if let Some(snapshot) = self
            .archive
            .as_ref()
            .and_then(|a| a.snapshots.get(&section))
            .cloned()
        {
            self.data.insert(section, snapshot.payload);
            self.loads.entry(section).or_default().updated = Some(snapshot.updated);
            if section == Section::Timetable {
                if let Ok(week) = NaiveDate::parse_from_str(&snapshot.week, "%Y-%m-%d") {
                    self.week = week;
                }
            }
            if section == Section::Messages {
                self.page = snapshot.page;
            }
        }
    }
    fn set_accent(&mut self, accent: Accent, ctx: &egui::Context) {
        self.accent = accent;
        theme::apply(ctx, accent);
        if !self.demo && accent.save().is_err() {
            self.status = tr!(
                "Kolor zmieniony. Nie udało się zapisać ustawienia wyglądu.",
                "Color changed, but the appearance setting could not be saved."
            )
            .into();
        }
    }
    fn sign_in(&mut self, ctx: egui::Context) {
        let credentials = vault::Credentials {
            login: self.login.trim().into(),
            password: std::mem::take(&mut self.password),
        };
        let remember = self.remember;
        self.auth_busy = true;
        self.epoch += 1;
        let epoch = self.epoch;
        let owner = credentials.login.clone();
        self.bind_cache(&owner);
        self.status = tr!("Logowanie do Librusa…", "Signing in to Librus…").into();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                if !remember {
                    vault::forget().map_err(|_| {
                        tr!(
                            "Nie można usunąć zapisanych danych logowania. Spróbuj ponownie.",
                            "Could not remove saved credentials. Please try again."
                        )
                        .to_owned()
                    })?;
                }
                let session = api::Session::login(&credentials.login, &credentials.password)
                    .map_err(|e| format!("{e:#}"))?;
                let warning = if remember {
                    vault::save(&credentials).err().map(|_|tr!("Połączono, ale nie udało się zapamiętać konta w Menedżerze poświadczeń Windows.", "Connected, but the account could not be saved in Windows Credential Manager.").into())
                } else {
                    None
                };
                Ok((Arc::new(session), warning))
            })();
            let _ = tx.send(Event::Login { epoch, result });
            ctx.request_repaint();
        });
    }
    fn clear_data(&mut self) {
        self.data.clear();
        self.loads.clear();
        self.selected = None;
        self.detail = None;
        self.detail_error = None;
        self.detail_loading = false;
        self.page = 1;
        self.view = views::ViewState::default();
    }
    fn fetch(&mut self, ctx: egui::Context) {
        let section = self.section;
        if section == Section::Account {
            return;
        }
        if self.demo {
            self.data = demo::payloads(self.week);
            self.status = tr!(
                "Odświeżono dane przykładowe. Tryb podglądu nie korzysta z sieci.",
                "Sample data refreshed. Preview mode does not use the network."
            )
            .into();
            return;
        }
        let Some(session) = self.session.clone() else {
            self.restore_snapshot(section);
            self.status = tr!(
                "Zaloguj się, aby odświeżyć zapisane dane.",
                "Sign in to refresh your saved data."
            )
            .into();
            self.section = Section::Account;
            return;
        };
        let state = self.loads.entry(section).or_default();
        state.loading = true;
        state.error = None;
        state.revision += 1;
        let revision = state.revision;
        let epoch = self.epoch;
        let week = self.week;
        let page = self.page;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = session
                .load(section, week, page)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Data {
                epoch,
                section,
                revision,
                result,
            });
            ctx.request_repaint();
        });
    }
    fn open_message(&mut self, id: String, ctx: egui::Context) {
        self.selected = Some(id.clone());
        self.detail = None;
        self.detail_error = None;
        if let Some(message) = self
            .archive
            .as_ref()
            .and_then(|a| a.details.get(&id))
            .cloned()
        {
            self.detail = Some(message);
        }
        if self.demo {
            if let Some(Payload::Messages { items, .. }) = self.data.get(&Section::Messages) {
                self.detail = items.iter().find(|m| m.id == id).cloned();
            }
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.detail_loading = true;
        let epoch = self.epoch;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = session.message_detail(&id).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Detail { epoch, id, result });
            ctx.request_repaint();
        });
    }
    fn sign_out(&mut self, ctx: egui::Context) {
        if self.demo {
            return;
        }
        let session = self.session.take();
        self.epoch += 1;
        let epoch = self.epoch;
        self.password.zeroize();
        self.clear_data();
        self.archive = None;
        let _ = cache::deactivate();
        self.auth_busy = true;
        self.section = Section::Account;
        self.status = tr!("Wylogowywanie…", "Signing out…").into();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result=vault::forget().map_err(|_|tr!("Sesję zamknięto, ale zapisanych danych nie usunięto. Usuń zapisane dane logowania aplikacji w Menedżerze poświadczeń Windows.", "Session closed, but saved credentials could not be removed. Remove the saved app sign-in entry from Windows Credential Manager.").to_owned());
            let remote = session.map(|s| s.logout()).transpose();
            let result=result.and_then(|_|remote.map(|_|()).map_err(|_|tr!("Wylogowano lokalnie i usunięto zapisane konto. Serwer nie potwierdził wylogowania.", "Signed out locally and removed the saved account. The server did not confirm sign-out.").into()));
            let _ = tx.send(Event::Logout { epoch, result });
            ctx.request_repaint();
        });
    }
    fn receive(&mut self, ctx: &egui::Context) {
        let mut initial_fetch = false;
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Login { epoch, result } if epoch == self.epoch => {
                    self.auth_busy = false;
                    match result {
                        Ok((session, warning)) => {
                            self.session = Some(session);
                            if warning.is_some() {
                                self.remember = false;
                            }
                            self.status = warning.unwrap_or_else(|| {
                                tr!(
                                    "Połączono z Librusem. Miłego dnia!",
                                    "Connected to Librus. Have a nice day!"
                                )
                                .into()
                            });
                            if let Some(archive) = &self.archive {
                                let _ = cache::activate(&archive.owner);
                                self.section = archive.last_section;
                            } else {
                                self.section = Section::Timetable;
                            }
                            initial_fetch = !self.data.contains_key(&self.section);
                        }
                        Err(e) => self.status = e,
                    }
                }
                Event::Data {
                    epoch,
                    section,
                    revision,
                    result,
                } if epoch == self.epoch => {
                    let state = self.loads.entry(section).or_default();
                    if state.revision != revision {
                        continue;
                    }
                    state.loading = false;
                    match result {
                        Ok(mut payload) => {
                            if let Payload::Grades {
                                behaviour,
                                behaviour_error: Some(_),
                                ..
                            } = &mut payload
                            {
                                if let Some(Payload::Grades { behaviour: old, .. }) =
                                    self.data.get(&section)
                                {
                                    for grade in old {
                                        if !behaviour.iter().any(|g| {
                                            g.value == grade.value
                                                && g.semester == grade.semester
                                                && g.date == grade.date
                                        }) {
                                            behaviour.push(grade.clone());
                                        }
                                    }
                                }
                            }
                            let count = payload.len();
                            let updated = chrono::Utc::now()
                                .with_timezone(&chrono_tz::Europe::Warsaw)
                                .format("%d.%m.%Y %H:%M")
                                .to_string();
                            state.updated = Some(updated.clone());
                            self.status = trf!(
                                "{} · pobrano {} wpisów",
                                "{} · downloaded {} entries",
                                section.title(),
                                count
                            );
                            self.persist(section, &payload, &updated);
                            self.data.insert(section, payload);
                        }
                        Err(e) => {
                            self.status = tr!(
                                "Nie udało się pobrać danych. Szczegóły w otwartej zakładce.",
                                "Could not download data. See the current tab for details."
                            )
                            .into();
                            state.error = Some(e);
                            if !self.data.contains_key(&section) {
                                self.restore_snapshot(section);
                            }
                        }
                    }
                }
                Event::Detail { epoch, id, result }
                    if epoch == self.epoch && self.selected.as_deref() == Some(&id) =>
                {
                    self.detail_loading = false;
                    match result {
                        Ok(message) => {
                            if let Some(archive) = &mut self.archive {
                                archive.details.insert(id.clone(), message.clone());
                                if archive.details.len() > 100 {
                                    archive.details.retain(|key, _| key == &id);
                                }
                                if let Some(cache::Snapshot {
                                    payload: Payload::Messages { items, .. },
                                    ..
                                }) = archive.snapshots.get_mut(&Section::Messages)
                                {
                                    if let Some(message) = items.iter_mut().find(|m| m.id == id) {
                                        message.unread = false;
                                    }
                                }
                                let _ = cache::save(archive);
                            }
                            self.detail = Some(message);
                            if let Some(Payload::Messages { items, .. }) =
                                self.data.get_mut(&Section::Messages)
                            {
                                if let Some(message) = items.iter_mut().find(|m| m.id == id) {
                                    message.unread = false;
                                }
                            }
                        }
                        Err(error) => self.detail_error = Some(error),
                    }
                }
                Event::Logout { epoch, result } if epoch == self.epoch => {
                    self.auth_busy = false;
                    self.remember = false;
                    self.status = result.err().unwrap_or_else(|| {
                        tr!(
                            "Wylogowano. Zapisane dane konta zostały usunięte.",
                            "Signed out. Saved account credentials have been removed."
                        )
                        .into()
                    });
                }
                _ => {}
            }
        }
        if initial_fetch {
            self.fetch(ctx.clone());
        }
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        let mut selected = self.section;
        let compact = ctx.screen_rect().height() < 760.;
        egui::SidePanel::left("sidebar")
            .exact_width(208.)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(18, 23, 32))
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_white_alpha(13)))
                    .inner_margin(20.),
            )
            .show(ctx, |ui| {
                if compact {
                    ui.spacing_mut().item_spacing.y = 5.;
                }
                ui.add_space(if compact { 2. } else { 11. });
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.), egui::Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(rect, 5., self.accent.color());
                    painter.line_segment(
                        [
                            rect.left_top() + egui::vec2(10., 7.),
                            rect.left_bottom() + egui::vec2(10., -7.),
                        ],
                        egui::Stroke::new(3.0_f32, Color32::from_rgb(12, 22, 36)),
                    );
                    painter.line_segment(
                        [
                            rect.left_bottom() + egui::vec2(10., -7.),
                            rect.right_bottom() + egui::vec2(-8., -7.),
                        ],
                        egui::Stroke::new(3.0_f32, Color32::from_rgb(12, 22, 36)),
                    );
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Klaso Desktop").size(23.).strong());
                        ui.small(
                            RichText::new(tr!("Twój dziennik szkolny", "Your school journal"))
                                .size(11.)
                                .color(MUTED),
                        );
                    });
                });
                ui.add_space(if compact { 15. } else { 24. });
                ui.small(RichText::new(tr!("Dziennik", "School journal")).color(MUTED));
                ui.add_space(8.);
                for section in [
                    Section::Timetable,
                    Section::Grades,
                    Section::Attendance,
                    Section::Messages,
                    Section::Notices,
                ] {
                    if nav(ui, section, self.section == section, self.accent) {
                        selected = section;
                    }
                }
                ui.add_space(15.);
                ui.separator();
                ui.add_space(10.);
                if nav(
                    ui,
                    Section::Account,
                    self.section == Section::Account,
                    self.accent,
                ) {
                    selected = Section::Account;
                }
                ui.add_space((ui.available_height() - if compact { 145. } else { 175. }).max(8.));
                ui.small(RichText::new(tr!("Kolor akcentu", "Accent color")).color(MUTED));
                ui.horizontal(|ui| {
                    for accent in Accent::ALL {
                        let (rect, response) =
                            ui.allocate_exact_size(Vec2::splat(30.), egui::Sense::click());
                        ui.painter()
                            .rect_filled(rect.shrink(3.), 4., accent.color());
                        if accent == self.accent {
                            ui.painter()
                                .rect_stroke(rect, 5., egui::Stroke::new(1.0_f32, TEXT));
                        }
                        if response.on_hover_text(accent.name()).clicked() {
                            self.set_accent(accent, ctx);
                        }
                    }
                });
                ui.small(RichText::new(self.accent.name()).color(MUTED));
                ui.add_space(13.);
                theme::inset().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(7.), egui::Sense::hover());
                        ui.painter().circle_filled(
                            r.center(),
                            3.,
                            if self.connected() { GREEN } else { MUTED },
                        );
                        ui.small(if self.demo {
                            tr!("Tryb podglądu", "Preview mode")
                        } else if self.connected() {
                            if self.session.as_ref().is_some_and(|s| s.is_authenticated()) {
                                tr!("Połączono z Synergią", "Connected to Synergia")
                            } else {
                                tr!("Gotowe do pobrania", "Ready to download")
                            }
                        } else {
                            if self.archive.is_some() {
                                tr!("Zapis offline", "Offline snapshot")
                            } else {
                                tr!("Nie zalogowano", "Not signed in")
                            }
                        });
                    });
                });
            });
        if selected != self.section {
            self.section = selected;
            self.view.search.clear();
            if self.connected()
                && selected != Section::Account
                && !self.data.contains_key(&selected)
                && !self.loads.get(&selected).is_some_and(|s| s.loading)
            {
                self.fetch(ctx.clone());
            }
        }
    }
    fn account(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let accent = self.accent;
        theme::glass(accent).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(tr!("Język", "Language")).size(21.).strong());
            ui.label(
                RichText::new(tr!(
                    "Wybierz język interfejsu. Zmiana działa od razu.",
                    "Choose the interface language. Changes apply immediately."
                ))
                .color(MUTED),
            );
            let mut language = self.language;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut language, Language::English, "English");
                ui.selectable_value(&mut language, Language::Polish, "Polski");
            });
            if language != self.language {
                self.language = language;
                language.activate();
                self.status = if !self.demo && language.save().is_err() {
                    tr!(
                        "Język zmieniony, ale nie udało się zapisać ustawienia.",
                        "Language changed, but the preference could not be saved."
                    )
                    .into()
                } else {
                    tr!(
                        "Język interfejsu został zmieniony.",
                        "Interface language updated."
                    )
                    .into()
                };
                ctx.request_repaint();
            }
        });
        ui.add_space(16.);
        theme::glass(accent).show(ui,|ui|{
            ui.set_min_width(ui.available_width());
            if self.connected(){
                theme::badge(ui,if self.demo{tr!("DANE PRZYKŁADOWE", "SAMPLE DATA")}else{tr!("KONTO GOTOWE", "ACCOUNT READY")},GREEN);ui.add_space(12.);ui.label(RichText::new(tr!("Połączenie z dziennikiem", "School journal connection")).size(27.).strong());ui.label(trf!("Konto Synergia: {}", "Synergia account: {}",self.login));ui.add_space(10.);
                if !self.demo{ui.label(if self.remember{tr!("Logowanie zostanie zapamiętane na tym komputerze.", "You will stay signed in on this computer.")}else{tr!("Konto nie jest zapisane. Po zamknięciu aplikacji zaloguj się ponownie.", "This account is not saved. Sign in again after closing the app.")});if ui.add_enabled(!self.auth_busy,egui::Button::new(tr!("Wyloguj i zapomnij konto", "Sign out and forget account"))).clicked(){self.sign_out(ctx.clone());}}
                else{ui.label(tr!("Ten podgląd nie loguje się do Librusa i nie zapisuje danych konta.", "This preview does not sign in to Librus or save account credentials."));}
            }else{
                ui.set_max_width(520.);ui.label(RichText::new(tr!("Zaloguj się do Synergii", "Sign in to Synergia")).size(28.).strong());ui.label(RichText::new(tr!("Zaloguj się kontem Synergia otrzymanym w szkole.", "Sign in with the Synergia account provided by your school.")).color(MUTED));ui.add_space(15.);
                ui.add_enabled_ui(!self.auth_busy,|ui|{
                    ui.label(tr!("Login Synergia", "Synergia username"));ui.add_sized([ui.available_width(),40.],egui::TextEdit::singleline(&mut self.login).hint_text(tr!("Login ze szkoły", "School username")));ui.add_space(4.);ui.label(tr!("Hasło", "Password"));let response=ui.add_sized([ui.available_width(),40.],egui::TextEdit::singleline(&mut self.password).password(true).hint_text(tr!("Twoje hasło", "Your password")));ui.add_space(5.);
                    if ui.checkbox(&mut self.remember,tr!("Zapamiętaj moje logowanie", "Keep me signed in")).changed() && !self.remember && vault::forget().is_err(){self.status=tr!("Nie można usunąć zapisanych danych. Sprawdź Menedżera poświadczeń Windows.", "Could not remove saved credentials. Check Windows Credential Manager.").into();}
                    ui.small(RichText::new(tr!("Dane logowania są przechowywane w Menedżerze poświadczeń Windows.", "Credentials are stored in Windows Credential Manager.")).color(MUTED));ui.add_space(12.);
                    let valid=!self.login.trim().is_empty() && !self.password.is_empty();
                    let clicked=ui.add_enabled(valid,egui::Button::new(RichText::new(tr!("Połącz z Librusem", "Connect to Librus")).strong().color(Color32::from_rgb(10,20,34))).fill(accent.color()).min_size(Vec2::new(200.,44.))).clicked();
                    if valid && (clicked || (response.lost_focus() && ui.input(|i|i.key_pressed(egui::Key::Enter)))){self.sign_in(ctx.clone());}
                });
                ui.add_space(12.);ui.hyperlink_to(tr!("Otwórz oficjalną Synergię", "Open the official Synergia website"), "https://synergia.librus.pl/");
                if self.auth_busy{ui.horizontal(|ui|{ui.spinner();ui.label(tr!("Logowanie…", "Signing in…"));});}
                else{ui.add_space(6.);ui.label(RichText::new(&self.status).color(MUTED));}
            }
        });
        ui.add_space(16.);
        theme::glass(accent).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new(tr!("Kolor akcentu", "Accent color"))
                    .size(21.)
                    .strong(),
            );
            ui.label(
                RichText::new(tr!(
                    "Wybierz kolor przycisków i wyróżnień.",
                    "Choose a color for buttons and highlights."
                ))
                .color(MUTED),
            );
            ui.add_space(10.);
            ui.horizontal_wrapped(|ui| {
                for a in Accent::ALL {
                    if ui
                        .add(
                            egui::Button::new(RichText::new(a.name()).color(a.color()))
                                .selected(self.accent == a)
                                .min_size(Vec2::new(110., 45.)),
                        )
                        .clicked()
                    {
                        self.set_accent(a, ctx);
                    }
                }
            });
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.receive(ctx);
        theme::background(ctx, self.accent);
        self.sidebar(ctx);
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(16, 20, 28))
                    .inner_margin(egui::Margin::symmetric(22., 10.)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if self.auth_busy || self.loads.values().any(|s| s.loading) {
                        ui.spinner();
                    }
                    ui.add(
                        egui::Label::new(RichText::new(&self.status).size(11.).color(MUTED))
                            .truncate(),
                    )
                    .on_hover_text(&self.status);
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::none().inner_margin(egui::Margin {
                left: 24.,
                right: 24.,
                top: 14.,
                bottom: 18.,
            }))
            .show(ctx, |ui| {
                views::heading(
                    ui,
                    self.section,
                    self.accent,
                    self.loads
                        .get(&self.section)
                        .and_then(|s| s.updated.as_deref()),
                    self.demo,
                );
                ui.add_space(6.);
                if self.section == Section::Account {
                    egui::ScrollArea::vertical()
                        .id_salt("account")
                        .show(ui, |ui| self.account(ui, ctx));
                    return;
                }
                if !self.connected() && !self.data.contains_key(&self.section) {
                    theme::glass(self.accent).show(ui, |ui| {
                        ui.label(tr!(
                            "Zaloguj się, aby zobaczyć swoje dane.",
                            "Sign in to view your data."
                        ));
                        if ui
                            .button(tr!("Przejdź do logowania", "Go to sign-in"))
                            .clicked()
                        {
                            self.section = Section::Account;
                        }
                    });
                    return;
                }
                let loading = self.loads.get(&self.section).is_some_and(|s| s.loading);
                let mut next_page = self.page;
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8., 4.);
                    ui.spacing_mut().button_padding = Vec2::new(10., 5.);
                    ui.spacing_mut().interact_size.y = 28.;
                    ui.style_mut()
                        .text_styles
                        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.));
                    ui.horizontal_wrapped(|ui| {
                        if self.section != Section::Timetable {
                            ui.add_sized(
                                [
                                    if self.section == Section::Grades {
                                        (ui.available_width() - 300.).clamp(140., 220.)
                                    } else {
                                        220.
                                    },
                                    28.,
                                ],
                                egui::TextEdit::singleline(&mut self.view.search)
                                    .hint_text(tr!("Szukaj w tej zakładce…", "Search this tab…")),
                            );
                        }
                        if self.section == Section::Timetable {
                            if ui
                                .button("‹")
                                .on_hover_text(tr!("Poprzedni tydzień", "Previous week"))
                                .clicked()
                            {
                                self.week -= Duration::weeks(1);
                                self.data.remove(&Section::Timetable);
                                self.fetch(ctx.clone());
                            }
                            if ui.button(tr!("Dzisiaj", "Today")).clicked() {
                                self.week = data::monday(data::today());
                                self.data.remove(&Section::Timetable);
                                self.fetch(ctx.clone());
                            }
                            if ui
                                .button("›")
                                .on_hover_text(tr!("Następny tydzień", "Next week"))
                                .clicked()
                            {
                                self.week += Duration::weeks(1);
                                self.data.remove(&Section::Timetable);
                                self.fetch(ctx.clone());
                            }
                            ui.label(format!(
                                "{} – {}",
                                self.week.format("%d.%m"),
                                (self.week + Duration::days(6)).format("%d.%m.%Y")
                            ));
                        }
                        if ui
                            .add_enabled(
                                !loading,
                                egui::Button::new(
                                    RichText::new(tr!("Odśwież", "Refresh"))
                                        .strong()
                                        .color(Color32::from_rgb(13, 20, 30)),
                                )
                                .fill(self.accent.color())
                                .min_size(Vec2::new(90., 28.)),
                            )
                            .clicked()
                        {
                            self.fetch(ctx.clone());
                        }
                        if loading {
                            ui.spinner();
                        }
                        if self.section == Section::Messages {
                            let total = match self.data.get(&self.section) {
                                Some(Payload::Messages { total, .. }) => *total,
                                _ => 0,
                            };
                            if ui
                                .add_enabled(!loading && self.page > 1, egui::Button::new("‹"))
                                .on_hover_text(tr!("Poprzednia strona", "Previous page"))
                                .clicked()
                            {
                                next_page -= 1;
                            }
                            ui.small(trf!(
                                "Strona {} · {} wiadomości",
                                "Page {} · {} messages",
                                self.page,
                                total
                            ));
                            if ui
                                .add_enabled(
                                    !loading && self.page * 20 < total && !self.demo,
                                    egui::Button::new("›"),
                                )
                                .on_hover_text(tr!("Następna strona", "Next page"))
                                .clicked()
                            {
                                next_page += 1;
                            }
                        }
                        if self.section == Section::Grades {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    views::behaviour(
                                        ui,
                                        self.data.get(&Section::Grades),
                                        self.accent,
                                    );
                                },
                            );
                        }
                    });
                });
                if next_page != self.page {
                    self.page = next_page;
                    self.selected = None;
                    self.detail = None;
                    self.detail_loading = false;
                    self.data.remove(&Section::Messages);
                    self.fetch(ctx.clone());
                }
                ui.add_space(8.);
                if let Some(error) = self.loads.get(&self.section).and_then(|s| s.error.as_ref()) {
                    theme::inset().show(ui, |ui| {
                        ui.label(
                            RichText::new(tr!(
                                "Nie udało się odświeżyć danych",
                                "Could not refresh data"
                            ))
                            .color(RED)
                            .strong(),
                        );
                        ui.label(error);
                        if self.data.contains_key(&self.section) {
                            ui.small(tr!(
                                "Poniżej znajdują się dane z ostatniego udanego pobrania.",
                                "Showing data from the last successful download below."
                            ));
                        }
                    });
                    ui.add_space(12.);
                }
                let mut clicked = None;
                egui::ScrollArea::vertical()
                    .id_salt(("page", self.section))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if let Some(payload) = self.data.get(&self.section) {
                            clicked = views::render(
                                ui,
                                payload,
                                self.accent,
                                &mut self.view,
                                views::Reader {
                                    selected: self.selected.as_deref(),
                                    detail: self.detail.as_ref(),
                                    error: self.detail_error.as_deref(),
                                    loading: self.detail_loading,
                                },
                                self.week,
                            );
                        } else {
                            views::unloaded(ui, self.section, self.accent, loading);
                        }
                    });
                if let Some(id) = clicked {
                    self.open_message(id, ctx.clone());
                }
            });
    }
}

fn nav(ui: &mut egui::Ui, section: Section, selected: bool, accent: Accent) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(
            ui.available_width(),
            if ui.ctx().screen_rect().height() < 760. {
                38.
            } else {
                46.
            },
        ),
        egui::Sense::click(),
    );
    let painter = ui.painter();
    if selected || response.hovered() {
        painter.rect_filled(
            rect,
            5.,
            if selected {
                accent.color().gamma_multiply(0.13)
            } else {
                Color32::from_white_alpha(6)
            },
        );
    }
    if selected {
        painter.rect_stroke(
            rect,
            5.,
            egui::Stroke::new(1.0_f32, accent.color().gamma_multiply(0.3)),
        );
        painter.rect_filled(
            egui::Rect::from_min_size(rect.left_top() + egui::vec2(0., 12.), Vec2::new(3., 22.)),
            1.,
            accent.color(),
        );
    }
    let color = if selected { accent.color() } else { MUTED };
    let icon =
        egui::Rect::from_min_size(rect.left_center() + egui::vec2(14., -10.), Vec2::splat(20.));
    paint_icon(painter, icon, section, color);
    painter.text(
        rect.left_center() + egui::vec2(46., 0.),
        egui::Align2::LEFT_CENTER,
        section.title(),
        egui::FontId::proportional(15.),
        if selected { TEXT } else { MUTED },
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}
fn paint_icon(p: &egui::Painter, r: egui::Rect, section: Section, color: Color32) {
    let stroke = egui::Stroke::new(1.5f32, color);
    let pos = |x: f32, y: f32| r.left_top() + egui::vec2(x, y);
    match section {
        Section::Timetable => {
            p.rect_stroke(r.shrink(1.), 2., stroke);
            for x in [7., 13.] {
                p.line_segment([pos(x, 2.), pos(x, 18.)], stroke);
            }
            for y in [7., 13.] {
                p.line_segment([pos(2., y), pos(18., y)], stroke);
            }
        }
        Section::Grades => {
            for (x, h) in [(2., 8.), (8., 16.), (14., 12.)] {
                p.rect_filled(
                    egui::Rect::from_min_size(pos(x, 19. - h), Vec2::new(4., h)),
                    1.,
                    color,
                );
            }
        }
        Section::Attendance => {
            p.rect_stroke(r.shrink(1.), 2., stroke);
            p.line_segment([pos(5., 10.), pos(9., 14.)], stroke);
            p.line_segment([pos(9., 14.), pos(16., 6.)], stroke);
        }
        Section::Messages => {
            p.rect_stroke(
                egui::Rect::from_min_max(pos(1., 3.), pos(19., 17.)),
                2.,
                stroke,
            );
            p.line_segment([pos(2., 4.), pos(10., 11.)], stroke);
            p.line_segment([pos(10., 11.), pos(18., 4.)], stroke);
        }
        Section::Notices => {
            p.line_segment([pos(2., 7.), pos(17., 2.)], stroke);
            p.line_segment([pos(17., 2.), pos(17., 17.)], stroke);
            p.line_segment([pos(17., 17.), pos(2., 12.)], stroke);
            p.line_segment([pos(2., 12.), pos(2., 7.)], stroke);
            p.line_segment([pos(5., 13.), pos(7., 19.)], stroke);
        }
        Section::Account => {
            p.circle_stroke(pos(10., 6.), 4., stroke);
            p.line_segment([pos(2., 19.), pos(3., 15.)], stroke);
            p.line_segment([pos(3., 15.), pos(10., 12.)], stroke);
            p.line_segment([pos(10., 12.), pos(17., 15.)], stroke);
            p.line_segment([pos(17., 15.), pos(18., 19.)], stroke);
        }
    }
}
fn main() -> eframe::Result {
    let compact = std::env::args().any(|arg| arg == "--compact");
    eframe::run_native(
        concat!("Klaso Desktop ", env!("CARGO_PKG_VERSION")),
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(if compact { [900., 640.] } else { [1260., 850.] })
                .with_min_inner_size([860., 620.]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_language_settings_are_available_before_and_after_login_in_both_languages() {
        for language in [Language::English, Language::Polish] {
            crate::i18n::with_language(language, || {
                for connected in [false, true] {
                    let mut app = cached_app();
                    app.language = language;
                    app.demo = connected;
                    let ctx = egui::Context::default();
                    theme::apply(&ctx, app.accent);
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                Vec2::new(1000., 1000.),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| app.account(ui, ctx));
                        },
                    );
                    fn collect(shape: &egui::Shape, text: &mut String) {
                        match shape {
                            egui::Shape::Text(value) => {
                                text.push_str(&value.galley.job.text);
                                text.push('\n');
                            }
                            egui::Shape::Vec(shapes) => {
                                for shape in shapes {
                                    collect(shape, text);
                                }
                            }
                            _ => {}
                        }
                    }
                    let mut text = String::new();
                    for shape in output.shapes {
                        collect(&shape.shape, &mut text);
                    }
                    assert!(text.contains(if language == Language::English {
                        "Language"
                    } else {
                        "Język"
                    }));
                    assert!(text.contains("English") && text.contains("Polski"));
                    assert!(text.contains(if language == Language::English {
                        "Accent color"
                    } else {
                        "Kolor akcentu"
                    }));
                    assert!(text.contains(if connected {
                        tr!("Połączenie z dziennikiem", "School journal connection")
                    } else {
                        tr!("Zaloguj się do Synergii", "Sign in to Synergia")
                    }));
                }
            });
        }
    }
    fn cached_app() -> App {
        let (tx, rx) = mpsc::channel();
        let week = data::monday(data::today());
        App {
            login: "fictional".into(),
            password: String::new(),
            remember: false,
            session: None,
            section: Section::Grades,
            accent: Accent::Cobalt,
            language: Language::English,
            status: "old".into(),
            auth_busy: false,
            epoch: 7,
            data: demo::payloads(week),
            loads: HashMap::from([(
                Section::Grades,
                LoadState {
                    loading: true,
                    error: None,
                    updated: Some("previous successful update".into()),
                    revision: 3,
                },
            )]),
            view: views::ViewState::default(),
            week,
            page: 1,
            selected: None,
            detail: None,
            detail_error: None,
            detail_loading: false,
            tx,
            rx,
            demo: false,
            archive: None,
        }
    }
    #[test]
    fn failed_refresh_preserves_previous_payload_and_timestamp() {
        let mut app = cached_app();
        let before = serde_json::to_value(app.data.get(&Section::Grades)).unwrap();
        app.tx
            .send(Event::Data {
                epoch: 7,
                section: Section::Grades,
                revision: 3,
                result: Err("offline".into()),
            })
            .unwrap();
        app.receive(&egui::Context::default());
        assert_eq!(
            before,
            serde_json::to_value(app.data.get(&Section::Grades)).unwrap()
        );
        let state = &app.loads[&Section::Grades];
        assert_eq!(state.updated.as_deref(), Some("previous successful update"));
        assert_eq!(state.error.as_deref(), Some("offline"));
        assert!(!state.loading);
    }
    #[test]
    fn late_replies_cannot_overwrite_another_account_or_newer_refresh() {
        let mut app = cached_app();
        let before = serde_json::to_value(app.data.get(&Section::Grades)).unwrap();
        for (epoch, revision) in [(6, 3), (7, 2)] {
            app.tx
                .send(Event::Data {
                    epoch,
                    section: Section::Grades,
                    revision,
                    result: Ok(Payload::Grades {
                        items: vec![],
                        behaviour: vec![],
                        behaviour_error: None,
                    }),
                })
                .unwrap();
        }
        app.receive(&egui::Context::default());
        assert_eq!(
            before,
            serde_json::to_value(app.data.get(&Section::Grades)).unwrap()
        );
    }
    #[test]
    fn failed_week_navigation_restores_the_last_successful_context() {
        let mut app = cached_app();
        let previous_week = app.week;
        let mut archive = cache::Archive::new("fictional".into());
        archive.snapshots.insert(
            Section::Timetable,
            cache::Snapshot {
                payload: app.data[&Section::Timetable].clone(),
                updated: "previous".into(),
                week: previous_week.to_string(),
                page: 1,
            },
        );
        app.archive = Some(archive);
        app.data.remove(&Section::Timetable);
        app.week += Duration::weeks(1);
        app.loads.insert(
            Section::Timetable,
            LoadState {
                loading: true,
                revision: 1,
                ..Default::default()
            },
        );
        app.tx
            .send(Event::Data {
                epoch: 7,
                section: Section::Timetable,
                revision: 1,
                result: Err("offline".into()),
            })
            .unwrap();
        app.receive(&egui::Context::default());
        assert_eq!(app.week, previous_week);
        assert!(app.data.contains_key(&Section::Timetable));
        assert_eq!(
            app.loads[&Section::Timetable].updated.as_deref(),
            Some("previous")
        );
    }
}
