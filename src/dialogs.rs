//! Native, accessible Win32 forms with a dark categorized Settings surface.
use crate::model::Settings;
use std::cell::{Cell, RefCell};
const WM_MOUSELEAVE: u32 = 0x02A3;
use windows::{
    core::{w, BOOL, PCWSTR},
    Win32::{
        Foundation::*,
        Graphics::{Dwm::*, Gdi::*},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{SetWindowTheme, DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED},
            Input::KeyboardAndMouse::{
                EnableWindow, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_UP,
            },
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
};

use crate::theme;
const BG: u32 = 0x0b1015;
const SIDEBAR: u32 = 0x101920;
const BORDER: u32 = 0x263640;
const CARD: u32 = 0x151f27;
const INPUT: u32 = 0x1e2c36;
const WHITE: u32 = 0xf3f8fb;
const MUTED: u32 = 0xa9bac6;
const GREEN: u32 = 0x38bdf8;
const SECTION_COUNT: usize = 11;
#[derive(Clone)]
enum Kind {
    Download,
    Text,
    Multiline,
    Number(f64, f64),
    Check,
    Choice(Vec<String>),
}
#[derive(Clone)]
struct Entry {
    label: String,
    value: String,
    kind: Kind,
    control: HWND,
}
impl Entry {
    fn text(label: &str, value: impl ToString) -> Self {
        Self {
            label: label.into(),
            value: value.to_string(),
            kind: Kind::Text,
            control: HWND::default(),
        }
    }
    fn number(label: &str, value: impl ToString, min: f64, max: f64) -> Self {
        Self {
            kind: Kind::Number(min, max),
            ..Self::text(label, value)
        }
    }
    fn check(label: &str, value: bool) -> Self {
        Self {
            kind: Kind::Check,
            ..Self::text(label, if value { "1" } else { "0" })
        }
    }
    fn choice(label: &str, value: usize, choices: &[&str]) -> Self {
        Self {
            kind: Kind::Choice(choices.iter().map(|s| s.to_string()).collect()),
            ..Self::text(label, value)
        }
    }
    fn choice_owned(label: &str, value: usize, choices: Vec<String>) -> Self {
        Self {
            kind: Kind::Choice(choices),
            ..Self::text(label, value)
        }
    }
}
struct Skin {
    background: HBRUSH,
    card: HBRUSH,
    input: HBRUSH,
    divider: HBRUSH,
    font: HFONT,
    small: HFONT,
    title: HFONT,
}
impl Skin {
    unsafe fn new() -> Self {
        unsafe {
            Self {
                background: CreateSolidBrush(rgb(BG)),
                card: CreateSolidBrush(rgb(CARD)),
                input: CreateSolidBrush(rgb(INPUT)),
                divider: CreateSolidBrush(rgb(BORDER)),
                font: font(15, 400),
                small: font(theme::CAPTION, 400),
                title: font(26, 600),
            }
        }
    }
}
impl Drop for Skin {
    fn drop(&mut self) {
        unsafe {
            for h in [
                self.background.0,
                self.card.0,
                self.input.0,
                self.divider.0,
                self.font.0,
                self.small.0,
                self.title.0,
            ] {
                let _ = DeleteObject(HGDIOBJ(h));
            }
        }
    }
}
enum DownloadEvent {
    Progress(u64, u64),
    Done(Result<(), String>),
}
struct Download {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    rx: std::sync::mpsc::Receiver<DownloadEvent>,
}
impl Download {
    fn start() -> Self {
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c = cancel.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::offline_translation::install(&c, |n, t| {
                let _ = tx.send(DownloadEvent::Progress(n, t));
            })
            .map_err(|e| e.to_string());
            let _ = tx.send(DownloadEvent::Done(result));
        });
        Self { cancel, rx }
    }
}
impl Drop for Download {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
#[derive(Clone, Debug, PartialEq)]
enum PackagePhase {
    Available,
    Connecting,
    Downloading(u64, u64),
    Installing,
    Canceling,
    Canceled,
    Ready,
    Failed,
}
struct PackageUi {
    status: String,
    action: &'static str,
    note: &'static str,
    percent: usize,
    busy: bool,
}
fn package_ui(phase: &PackagePhase, en: bool) -> PackageUi {
    let l = |pt, en_text| if en { en_text } else { pt };
    let busy = matches!(
        phase,
        PackagePhase::Connecting
            | PackagePhase::Downloading(..)
            | PackagePhase::Installing
            | PackagePhase::Canceling
    );
    let (status, action, percent) = match phase {
        PackagePhase::Available => (
            l(
                "Pacote opcional · cerca de 200 MB",
                "Optional package · about 200 MB",
            )
            .into(),
            l("Baixar e ativar", "Download and enable"),
            0,
        ),
        PackagePhase::Connecting => (
            l("Preparando download…", "Preparing download…").into(),
            l("Cancelar download", "Cancel download"),
            0,
        ),
        PackagePhase::Downloading(n, total) => {
            let percent = if *total == 0 {
                0
            } else {
                n.saturating_mul(100)
                    .checked_div(*total)
                    .unwrap_or(0)
                    .min(100) as usize
            };
            (
                format!(
                    "{} {}% · {:.1} / {:.1} MB",
                    l("Baixando", "Downloading"),
                    percent,
                    *n as f64 / 1e6,
                    *total as f64 / 1e6
                ),
                l("Cancelar download", "Cancel download"),
                percent,
            )
        }
        PackagePhase::Installing => (
            l(
                "Download concluído. Verificando e instalando…",
                "Download complete. Verifying and installing…",
            )
            .into(),
            l("Cancelar download", "Cancel download"),
            100,
        ),
        PackagePhase::Canceling => (
            l("Cancelando download…", "Canceling download…").into(),
            l("Cancelando…", "Canceling…"),
            0,
        ),
        PackagePhase::Canceled => (
            l(
                "Download cancelado. Você pode tentar novamente.",
                "Download canceled. You can try again.",
            )
            .into(),
            l("Continuar download", "Continue download"),
            0,
        ),
        PackagePhase::Ready => (
            l(
                "Pacote instalado · pronto para usar",
                "Package installed · ready to use",
            )
            .into(),
            l("Pacote instalado", "Package installed"),
            100,
        ),
        PackagePhase::Failed => (
            l(
                "Não foi possível concluir. Verifique a conexão e o espaço livre.",
                "Could not finish. Check your connection and free disk space.",
            )
            .into(),
            l("Tentar novamente", "Try again"),
            0,
        ),
    };
    let note = if busy {
        l(
            "Mantenha esta janela aberta. Fechar cancela o download.",
            "Keep this window open. Closing cancels the download.",
        )
    } else if matches!(phase, PackagePhase::Ready) {
        l(
            "Use a opção acima e salve para aplicar. O pacote fica no computador.",
            "Use the option above and save to apply. The package stays on this computer.",
        )
    } else {
        l(
            "Ao concluir, a opção será marcada. Salve para ativar.",
            "The option will be checked when complete. Save to enable.",
        )
    };
    PackageUi {
        status,
        action,
        note,
        percent,
        busy,
    }
}
unsafe fn refresh_package(hwnd: HWND, form: &Form) {
    unsafe {
        use windows::Win32::UI::Controls::{PBM_SETBARCOLOR, PBM_SETBKCOLOR, PBM_SETPOS};
        let phase = form.package_phase.borrow();
        let ui = package_ui(&phase, form.english);
        for (id, text) in [
            (4000, ui.status.as_str()),
            (124, ui.action),
            (4002, ui.note),
        ] {
            if let Ok(control) = GetDlgItem(Some(hwnd), id) {
                let _ = SetWindowTextW(control, &windows::core::HSTRING::from(text));
            }
        }
        if let Ok(progress) = GetDlgItem(Some(hwnd), 4001) {
            SendMessageW(progress, PBM_SETPOS, Some(WPARAM(ui.percent)), None);
            SendMessageW(
                progress,
                PBM_SETBARCOLOR,
                None,
                Some(LPARAM(rgb(GREEN).0 as isize)),
            );
            SendMessageW(
                progress,
                PBM_SETBKCOLOR,
                None,
                Some(LPARAM(rgb(INPUT).0 as isize)),
            );
        }
        if let Ok(button) = GetDlgItem(Some(hwnd), 124) {
            let _ = EnableWindow(
                button,
                !matches!(*phase, PackagePhase::Ready | PackagePhase::Canceling),
            );
        }
        if let Ok(toggle) = GetDlgItem(Some(hwnd), 123) {
            let ready = matches!(*phase, PackagePhase::Ready);
            let _ = EnableWindow(toggle, ready);
            if !ready {
                SendMessageW(toggle, BM_SETCHECK, Some(WPARAM(0)), None);
            }
        }
        if let Ok(save) = GetDlgItem(Some(hwnd), 1) {
            let _ = EnableWindow(save, !ui.busy);
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}
struct Form {
    package_phase: RefCell<PackagePhase>,
    download: RefCell<Option<Download>>,
    entries: RefCell<Vec<Entry>>,
    done: Cell<bool>,
    accepted: Cell<bool>,
    english: bool,
    categorized: bool,
    section: Cell<usize>,
    rows: RefCell<Vec<Vec<HWND>>>,
    sidebar_buttons: RefCell<Vec<HWND>>,
    hovered_btn: Cell<Option<i32>>,
    skin: Skin,
    task: Option<RefCell<TaskForm>>,
    validation_error: RefCell<Option<(usize, String)>>,
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn rgb(c: u32) -> COLORREF {
    COLORREF(((c >> 16) & 255) | (c & 0xff00) | ((c & 255) << 16))
}
unsafe fn font(size: i32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -size,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        )
    }
}
fn category(i: usize) -> usize {
    match i {
        0 | 14 | 15 => 0,
        1 | 2 | 12 => 1,
        10 | 11 | 13 => 2,
        16 => 3,
        3..=5 => 4,
        6..=9 => 5,
        17 => 6,
        22..=24 => 8,
        25 | 26 => 9,
        _ => 7,
    }
}
fn sections(en: bool) -> [&'static str; SECTION_COUNT] {
    if en {
        [
            "General",
            "Display",
            "Appearance",
            "Calendar",
            "Pomodoro",
            "Hydration",
            "Shelf",
            "System alerts",
            "Lyrics",
            "Lyrics display",
            "About",
        ]
    } else {
        [
            "Geral",
            "Tela",
            "Aparência",
            "Calendário",
            "Pomodoro",
            "Hidratação",
            "Prateleira",
            "Avisos do sistema",
            "Letras",
            "Leitura de letras",
            "Sobre",
        ]
    }
}
fn subtitle(section: usize, en: bool) -> &'static str {
    let pt = [
        "Pequenos ajustes para seu dia.",
        "No lugar certo, em qualquer tela.",
        "Uma notch que combina com você.",
        "Sua semana, organizada.",
        "Encontre seu ritmo de foco.",
        "Um lembrete para cuidar de você.",
        "Arquivos e ideias ao alcance.",
        "Só o que você precisa saber.",
        "Tradução offline e preferências de busca.",
        "Animação e tradução durante a leitura.",
        "Uma pequena companhia para o Windows.",
    ];
    let eng = [
        "Small adjustments for your day.",
        "At home on every display.",
        "Make the notch feel like yours.",
        "Make room for your week.",
        "Find your focus rhythm.",
        "A little reminder to look after yourself.",
        "Keep files and ideas within reach.",
        "Just the information you need.",
        "Offline translation and search preferences.",
        "Animation and translation while reading.",
        "A little companion for Windows.",
    ];
    if en {
        eng[section]
    } else {
        pt[section]
    }
}
fn hint(index: usize, en: bool) -> &'static str {
    match (index, en) {
        (0, false) => "Textos da interface",
        (0, true) => "Interface language",
        (1, false) => "0 usa o monitor principal",
        (1, true) => "0 selects the primary display",
        (2, false) => "De 0,75 a 2 (1 = tamanho padrão)",
        (2, true) => "From 0.75 to 2 (1 = default size)",
        (3, false) => "Duração de cada sessão de foco",
        (3, true) => "Duration of each focus session",
        (4 | 5, false) => "Intervalo entre sessões, em minutos",
        (4 | 5, true) => "Break between sessions, in minutes",
        (6, false) => "Quantidade diária, em mililitros",
        (6, true) => "Daily amount, in milliliters",
        (7, false) => "Quantidade por registro de água",
        (7, true) => "Amount per water entry",
        (8 | 9, false) => "Escolha um horário na lista",
        (8 | 9, true) => "Choose a time from the list",
        (10, false) => "Desative para abrir ao passar o mouse",
        (10, true) => "Turn off to open on hover",
        (11, false) => "Uma pílula menor quando tudo está quieto",
        (11, true) => "A smaller pill when nothing is happening",
        (12, false) => "Passe o mouse na linha da borda para revelar",
        (12, true) => "Hover over the edge handle to reveal",
        (13, false) => "Transições simples e suaves",
        (13, true) => "Simple, gentle transitions",
        (14, false) => "Sons para foco, tarefas e água",
        (14, true) => "Chimes for focus, tasks and water",
        (15, false) => "Abrir quando você entrar no Windows",
        (15, true) => "Open when you sign in to Windows",
        (16, false) => "Primeiro dia da semana",
        (16, true) => "First day of the week",
        (17, false) => "Começar uma nova sessão com a Prateleira vazia",
        (17, true) => "Start each session with an empty Shelf",
        (18, false) => "Mostrar nível de carga na barra",
        (18, true) => "Show battery level in the bar",
        (19..=21, false) => "Mostrar quando houver uma mudança",
        (19..=21, true) => "Show when something changes",
        (23, false) => "Baixe o pacote abaixo para usar sem internet.",
        (23, true) => "Download the package below to use without internet.",
        (25, false) => "Transição do verso em foco na notch",
        (25, true) => "Focused verse transition in the notch",
        (26, false) => "Desative para mostrar só no verso atual",
        (26, true) => "Turn off to show only on the current verse",
        (22, false) => "Uma expressão por linha. Maiúsculas e minúsculas são equivalentes.",
        (22, true) => "One phrase per line. Uppercase and lowercase are equivalent.",
        (_, false) => "Disponível sem assinatura",
        (_, true) => "Available without a subscription",
    }
}
unsafe fn text_draw(
    dc: HDC,
    text: &str,
    rect: RECT,
    color: u32,
    face: HFONT,
    flags: DRAW_TEXT_FORMAT,
) {
    unsafe {
        let old = SelectObject(dc, HGDIOBJ(face.0));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, rgb(color));
        let mut s: Vec<u16> = text.encode_utf16().collect();
        let mut r = rect;
        DrawTextW(dc, &mut s, &mut r, flags);
        SelectObject(dc, old);
    }
}
unsafe fn round(dc: HDC, rect: RECT, color: u32, radius: i32) {
    unsafe {
        let brush = CreateSolidBrush(rgb(color));
        let old_b = SelectObject(dc, HGDIOBJ(brush.0));
        let old_p = SelectObject(dc, GetStockObject(NULL_PEN));
        let _ = RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(dc, old_b);
        SelectObject(dc, old_p);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}
// Keep fills inexpensive: reuse the GDI DC brush rather than allocating per scanline.
unsafe fn paint_notch_surface(dc: HDC, bounds: RECT) {
    unsafe {
        let brush = HBRUSH(GetStockObject(DC_BRUSH).0);
        let height = (bounds.bottom - bounds.top).max(1);
        for y in (bounds.top..bounds.bottom).step_by(4) {
            let t = ((y - bounds.top) as f32 / height as f32).powi(3);
            let channel = |a: f32, b: f32| (a + (b - a) * t).round() as u32;
            let color = (channel(11., 15.) << 16) | (channel(16., 36.) << 8) | channel(21., 47.);
            SetDCBrushColor(dc, rgb(color));
            FillRect(
                dc,
                &RECT {
                    left: bounds.left,
                    top: y,
                    right: bounds.right,
                    bottom: (y + 4).min(bounds.bottom),
                },
                brush,
            );
        }
    }
}
unsafe fn panel(dc: HDC, rect: RECT, color: u32, radius: i32) {
    unsafe {
        round(dc, rect, BORDER, radius);
        let inner = RECT {
            left: rect.left + 1,
            top: rect.top + 1,
            right: rect.right - 1,
            bottom: rect.bottom - 1,
        };
        round(dc, inner, color, radius - 2);
    }
}
unsafe fn show_section(hwnd: HWND, form: &Form, section: usize) {
    unsafe {
        form.section.set(section);
        let rows = form.rows.borrow().clone();
        for (i, controls) in rows.iter().enumerate() {
            for control in controls {
                let _ = ShowWindow(
                    *control,
                    if category(i) == section {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
        }
        for btn in form.sidebar_buttons.borrow().iter() {
            let _ = InvalidateRect(Some(*btn), None, true);
            let _ = UpdateWindow(*btn);
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}
unsafe fn paint_form(hwnd: HWND, form: &Form) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut bounds = RECT::default();
        let _ = GetClientRect(hwnd, &mut bounds);
        FillRect(dc, &bounds, form.skin.background);
        if form.categorized {
            paint_notch_surface(dc, bounds);
            panel(
                dc,
                RECT {
                    left: 8,
                    top: 14,
                    right: 200,
                    bottom: bounds.bottom - 14,
                },
                SIDEBAR,
                28,
            );
            let v_div = RECT {
                left: 206,
                top: 20,
                right: 207,
                bottom: bounds.bottom - 20,
            };
            FillRect(dc, &v_div, form.skin.divider);

            let h_div = RECT {
                left: 216,
                top: bounds.bottom - 60,
                right: bounds.right - 24,
                bottom: bounds.bottom - 59,
            };
            FillRect(dc, &h_div, form.skin.divider);

            text_draw(
                dc,
                if form.english {
                    "Changes apply when you save."
                } else {
                    "As alterações são aplicadas ao salvar."
                },
                RECT {
                    left: 224,
                    top: bounds.bottom - 46,
                    right: bounds.right - 270,
                    bottom: bounds.bottom - 18,
                },
                MUTED,
                form.skin.small,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
            text_draw(
                dc,
                "Nivumi",
                RECT {
                    left: 22,
                    top: 24,
                    right: 180,
                    bottom: 58,
                },
                WHITE,
                form.skin.title,
                DT_LEFT | DT_SINGLELINE,
            );
            text_draw(
                dc,
                if form.english {
                    "Your Nivumi, your way"
                } else {
                    "Seu Nivumi, do seu jeito"
                },
                RECT {
                    left: 24,
                    top: 64,
                    right: 190,
                    bottom: 85,
                },
                MUTED,
                form.skin.small,
                DT_LEFT | DT_SINGLELINE,
            );
            text_draw(
                dc,
                &format!("REBUILD  {}", env!("CARGO_PKG_VERSION")),
                RECT {
                    left: 24,
                    top: bounds.bottom - 28,
                    right: 190,
                    bottom: bounds.bottom - 8,
                },
                theme::MUTED,
                form.skin.small,
                DT_LEFT | DT_SINGLELINE,
            );
            let s = form.section.get();
            text_draw(
                dc,
                sections(form.english)[s],
                RECT {
                    left: 222,
                    top: 25,
                    right: 770,
                    bottom: 60,
                },
                WHITE,
                form.skin.title,
                DT_LEFT | DT_SINGLELINE,
            );
            text_draw(
                dc,
                subtitle(s, form.english),
                RECT {
                    left: 224,
                    top: 62,
                    right: 770,
                    bottom: 85,
                },
                MUTED,
                form.skin.small,
                DT_LEFT | DT_SINGLELINE,
            );
            let entries = form.entries.borrow().clone();
            let validation_error = form.validation_error.borrow().clone();
            for (row, (i, _)) in entries
                .iter()
                .enumerate()
                .filter(|(i, _)| category(*i) == s)
                .enumerate()
            {
                let y = if s == 8 {
                    match i {
                        23 => 100,
                        24 => 204,
                        _ => 410,
                    }
                } else {
                    100 + row as i32 * theme::ROW_PITCH
                };
                panel(
                    dc,
                    RECT {
                        left: 216,
                        top: y,
                        right: 770,
                        bottom: y + if s == 8 {
                            match i {
                                23 => 92,
                                24 => 198,
                                _ => 190,
                            }
                        } else if matches!(entries[i].kind, Kind::Multiline) {
                            316
                        } else {
                            theme::ROW_HEIGHT
                        },
                    },
                    CARD,
                    22,
                );
                if matches!(
                    entries[i].kind,
                    Kind::Text | Kind::Number(..) | Kind::Multiline
                ) {
                    let control = entries[i].control;
                    let mut rect = RECT::default();
                    if GetWindowRect(control, &mut rect).is_ok() {
                        let mut point = POINT {
                            x: rect.left,
                            y: rect.top,
                        };
                        let _ = ScreenToClient(hwnd, &mut point);
                        let rect = RECT {
                            left: point.x - 8,
                            top: point.y - 6,
                            right: point.x + (rect.right - rect.left) + 8,
                            bottom: point.y + (rect.bottom - rect.top) + 6,
                        };
                        let border =
                            if windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() == control {
                                GREEN
                            } else {
                                BORDER
                            };
                        round(dc, rect, border, 16);
                        round(
                            dc,
                            RECT {
                                left: rect.left + 1,
                                top: rect.top + 1,
                                right: rect.right - 1,
                                bottom: rect.bottom - 1,
                            },
                            INPUT,
                            14,
                        );
                    }
                }
                if !matches!(entries[i].kind, Kind::Check | Kind::Download) {
                    let error = validation_error
                        .as_ref()
                        .filter(|(index, _)| *index == i)
                        .map(|(_, message)| message.as_str());
                    text_draw(
                        dc,
                        error.unwrap_or_else(|| hint(i, form.english)),
                        RECT {
                            left: 230,
                            top: y + 33,
                            right: if matches!(entries[i].kind, Kind::Multiline) {
                                750
                            } else {
                                550
                            },
                            bottom: y + 52,
                        },
                        if error.is_some() { theme::ERROR } else { MUTED },
                        form.skin.small,
                        DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
                    );
                }
            }
            if s == SECTION_COUNT - 1 {
                panel(
                    dc,
                    RECT {
                        left: 216,
                        top: 100,
                        right: 770,
                        bottom: 270,
                    },
                    CARD,
                    22,
                );
                let title_text = format!("Nivumi  v{}", env!("CARGO_PKG_VERSION"));
                text_draw(
                    dc,
                    &title_text,
                    RECT {
                        left: 236,
                        top: 120,
                        right: 748,
                        bottom: 154,
                    },
                    WHITE,
                    form.skin.title,
                    DT_LEFT | DT_SINGLELINE,
                );
                let body = if form.english {
                    "Your companion for music, focus and daily routines.\nChanges take effect after saving.\nYour preferences are stored securely on this computer."
                } else {
                    "Sua companhia para música, foco e rotina.\nAs alterações entram em vigor ao salvar.\nSuas preferências ficam salvas com segurança neste computador."
                };
                text_draw(
                    dc,
                    body,
                    RECT {
                        left: 236,
                        top: 164,
                        right: 748,
                        bottom: 250,
                    },
                    MUTED,
                    form.skin.font,
                    DT_LEFT | DT_WORDBREAK,
                );
            }
        }
        let _ = EndPaint(hwnd, &ps);
    }
}
unsafe fn paint_toggle(hwnd: HWND, dc: HDC, form: &Form) {
    unsafe {
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        FillRect(dc, &rect, form.skin.card);
        let i = (GetDlgCtrlID(hwnd) - 100).max(0) as usize;
        let mut buf = vec![0; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let len = GetWindowTextW(hwnd, &mut buf);
        let label = String::from_utf16_lossy(&buf[..len as usize]);
        text_draw(
            dc,
            &label,
            RECT {
                left: 12,
                top: 4,
                right: rect.right - 70,
                bottom: 27,
            },
            WHITE,
            form.skin.font,
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        if form.categorized {
            text_draw(
                dc,
                if i == 23 && *form.package_phase.borrow() == PackagePhase::Ready {
                    if form.english {
                        "Human translations always take priority."
                    } else {
                        "Traduções humanas sempre têm prioridade."
                    }
                } else {
                    hint(i, form.english)
                },
                RECT {
                    left: 12,
                    top: 28,
                    right: rect.right - 70,
                    bottom: 46,
                },
                MUTED,
                form.skin.small,
                DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        }
        let on = SendMessageW(hwnd, BM_GETCHECK, None, None).0 == 1;
        let cy = (rect.bottom - 22) / 2;
        let toggle = RECT {
            left: rect.right - 57,
            top: cy,
            right: rect.right - 13,
            bottom: cy + 22,
        };
        round(dc, toggle, if on { GREEN } else { theme::BORDER }, 22);
        let cx = if on {
            toggle.right - 19
        } else {
            toggle.left + 3
        };
        round(
            dc,
            RECT {
                left: cx,
                top: cy + 3,
                right: cx + 16,
                bottom: cy + 19,
            },
            if on { theme::SURFACE } else { WHITE },
            16,
        );
    }
}
unsafe extern "system" fn toggle_procedure(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(toggle_procedure), id);
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        if msg == WM_ERASEBKGND {
            return LRESULT(1);
        }
        if msg == WM_SETCURSOR {
            let _ = SetCursor(Some(LoadCursorW(None, IDC_HAND).unwrap_or_default()));
            return LRESULT(1);
        }
        if msg == WM_PAINT {
            if let Ok(parent) = GetParent(hwnd) {
                let ptr = GetWindowLongPtrW(parent, GWLP_USERDATA) as *const Form;
                if !ptr.is_null() {
                    let mut ps = PAINTSTRUCT::default();
                    let dc = BeginPaint(hwnd, &mut ps);
                    paint_toggle(hwnd, dc, &*ptr);
                    let _ = EndPaint(hwnd, &ps);
                    return LRESULT(0);
                }
            }
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        if matches!(
            msg,
            WM_LBUTTONUP | WM_KEYUP | WM_SETFOCUS | WM_KILLFOCUS | BM_SETCHECK
        ) {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        result
    }
}
unsafe fn paint_button(item: &DRAWITEMSTRUCT, form: &Form) {
    unsafe {
        let sidebar = (500..500 + SECTION_COUNT as u32).contains(&item.CtlID);
        let selected = sidebar && form.section.get() == (item.CtlID - 500) as usize;
        let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
        let hovered = form.hovered_btn.get() == Some(item.CtlID as i32);
        let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
        let disabled = item.itemState.0 & windows::Win32::UI::Controls::ODS_DISABLED.0 != 0;
        let color = if disabled {
            CARD
        } else if item.CtlID == 1 || item.CtlID == 124 {
            if pressed {
                0x2087b1
            } else if hovered {
                0x68d0fb
            } else {
                GREEN
            }
        } else if item.CtlID == 2 {
            if pressed {
                theme::CARD
            } else if hovered {
                theme::INPUT
            } else {
                theme::CARD
            }
        } else if sidebar {
            if pressed {
                if selected {
                    theme::PRESSED
                } else {
                    theme::CARD
                }
            } else if selected {
                if hovered {
                    theme::PRESSED
                } else {
                    theme::SELECTED
                }
            } else if hovered || focused {
                theme::CARD
            } else {
                SIDEBAR
            }
        } else if pressed {
            theme::PRESSED
        } else if hovered {
            theme::CARD
        } else {
            BG
        };
        if sidebar {
            let brush = HBRUSH(GetStockObject(DC_BRUSH).0);
            SetDCBrushColor(item.hDC, rgb(SIDEBAR));
            FillRect(item.hDC, &item.rcItem, brush);
        }
        round(item.hDC, item.rcItem, color, 20);
        let text = if sidebar {
            sections(form.english)[(item.CtlID - 500) as usize].to_string()
        } else if item.CtlID == 1 {
            if form.categorized {
                if form.english {
                    "Save changes"
                } else {
                    "Salvar alterações"
                }
            } else if form.english {
                "Save"
            } else {
                "Salvar"
            }
            .to_string()
        } else if item.CtlID == 124 {
            let mut buf = vec![0; GetWindowTextLengthW(item.hwndItem).max(0) as usize + 1];
            let n = GetWindowTextW(item.hwndItem, &mut buf);
            String::from_utf16_lossy(&buf[..n as usize])
        } else {
            if form.english { "Cancel" } else { "Cancelar" }.to_string()
        };
        if selected {
            let mut marker = item.rcItem;
            marker.left += 10;
            marker.right = marker.left + 5;
            marker.top = (marker.top + marker.bottom) / 2 - 2;
            marker.bottom = marker.top + 5;
            round(item.hDC, marker, GREEN, 3);
        }
        let mut r = item.rcItem;
        if sidebar {
            r.left += 24;
        }
        let text_color = if disabled {
            MUTED
        } else if item.CtlID == 1 || item.CtlID == 124 {
            theme::SURFACE
        } else if item.CtlID == 2 {
            if hovered {
                WHITE
            } else {
                theme::MUTED
            }
        } else if sidebar {
            if selected {
                WHITE
            } else if hovered {
                theme::TEXT
            } else {
                theme::MUTED
            }
        } else {
            WHITE
        };
        text_draw(
            item.hDC,
            &text,
            r,
            text_color,
            form.skin.font,
            DT_SINGLELINE | DT_VCENTER | if sidebar { DT_LEFT } else { DT_CENTER },
        );
    }
}
pub(super) unsafe extern "system" fn button_procedure(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(button_procedure), id);
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        if msg == WM_SETCURSOR {
            let _ = SetCursor(Some(LoadCursorW(None, IDC_HAND).unwrap_or_default()));
            return LRESULT(1);
        }
        if msg == WM_KEYDOWN {
            let key = wp.0 as u32;
            let ctl_id = GetDlgCtrlID(hwnd);
            let sidebar = (500..500 + SECTION_COUNT as i32).contains(&ctl_id);
            if sidebar {
                if key == VK_DOWN.0 as u32 {
                    let next_id = if ctl_id < 500 + SECTION_COUNT as i32 - 1 {
                        ctl_id + 1
                    } else {
                        500
                    };
                    if let Ok(parent) = GetParent(hwnd) {
                        if let Ok(next_btn) = GetDlgItem(Some(parent), next_id) {
                            let _ = SetFocus(Some(next_btn));
                            let ptr = GetWindowLongPtrW(parent, GWLP_USERDATA) as *const Form;
                            if !ptr.is_null() {
                                show_section(parent, &*ptr, (next_id - 500) as usize);
                            }
                            return LRESULT(0);
                        }
                    }
                } else if key == VK_UP.0 as u32 {
                    let prev_id = if ctl_id > 500 {
                        ctl_id - 1
                    } else {
                        500 + SECTION_COUNT as i32 - 1
                    };
                    if let Ok(parent) = GetParent(hwnd) {
                        if let Ok(prev_btn) = GetDlgItem(Some(parent), prev_id) {
                            let _ = SetFocus(Some(prev_btn));
                            let ptr = GetWindowLongPtrW(parent, GWLP_USERDATA) as *const Form;
                            if !ptr.is_null() {
                                show_section(parent, &*ptr, (prev_id - 500) as usize);
                            }
                            return LRESULT(0);
                        }
                    }
                }
            }
        }
        if msg == WM_MOUSEMOVE {
            if let Ok(parent) = GetParent(hwnd) {
                let ptr = GetWindowLongPtrW(parent, GWLP_USERDATA) as *const Form;
                if !ptr.is_null() {
                    let form = &*ptr;
                    let ctl_id = GetDlgCtrlID(hwnd);
                    if form.hovered_btn.get() != Some(ctl_id) {
                        let prev = form.hovered_btn.get();
                        form.hovered_btn.set(Some(ctl_id));
                        if let Some(prev_id) = prev {
                            if let Ok(prev_hwnd) = GetDlgItem(Some(parent), prev_id) {
                                let _ = InvalidateRect(Some(prev_hwnd), None, true);
                            }
                        }
                        let _ = InvalidateRect(Some(hwnd), None, true);
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut tme);
                    }
                }
            }
        }
        if msg == WM_MOUSELEAVE {
            if let Ok(parent) = GetParent(hwnd) {
                let ptr = GetWindowLongPtrW(parent, GWLP_USERDATA) as *const Form;
                if !ptr.is_null() {
                    let form = &*ptr;
                    let ctl_id = GetDlgCtrlID(hwnd);
                    if form.hovered_btn.get() == Some(ctl_id) {
                        form.hovered_btn.set(None);
                        let _ = InvalidateRect(Some(hwnd), None, true);
                    }
                }
            }
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Form;
        if !ptr.is_null() {
            let form = &*ptr;
            if let Some(task) = &form.task {
                if msg == WM_COMMAND {
                    if let Ok(mut task) = task.try_borrow_mut() {
                        task_command(hwnd, form, &mut task, wp);
                    }
                    return LRESULT(0);
                }
            }
            match msg {
                WM_ERASEBKGND => return LRESULT(1),
                WM_PAINT => {
                    paint_form(hwnd, form);
                    return LRESULT(0);
                }
                WM_DRAWITEM => {
                    if lp.0 != 0 && form.task.is_some() {
                        let item = &*(lp.0 as *const DRAWITEMSTRUCT);
                        if item.CtlID == 104 {
                            FillRect(item.hDC, &item.rcItem, form.skin.input);
                            let index = item.itemID as usize;
                            if let Some(color) = task_form::COLORS.get(index) {
                                round(
                                    item.hDC,
                                    RECT {
                                        left: item.rcItem.left + 8,
                                        top: item.rcItem.top + 5,
                                        right: item.rcItem.left + 28,
                                        bottom: item.rcItem.bottom - 5,
                                    },
                                    *color,
                                    5,
                                );
                            }
                            if item.itemID != u32::MAX {
                                let len = SendMessageW(
                                    item.hwndItem,
                                    CB_GETLBTEXTLEN,
                                    Some(WPARAM(index)),
                                    None,
                                )
                                .0;
                                if len >= 0 {
                                    let mut buf = vec![0u16; len as usize + 1];
                                    SendMessageW(
                                        item.hwndItem,
                                        CB_GETLBTEXT,
                                        Some(WPARAM(index)),
                                        Some(LPARAM(buf.as_mut_ptr() as isize)),
                                    );
                                    let mut rect = item.rcItem;
                                    rect.left += 38;
                                    text_draw(
                                        item.hDC,
                                        &String::from_utf16_lossy(&buf[..len as usize]),
                                        rect,
                                        WHITE,
                                        form.skin.font,
                                        DT_SINGLELINE | DT_VCENTER,
                                    );
                                }
                            }
                            if item.itemState.0 & (ODS_FOCUS.0 | ODS_SELECTED.0) != 0 {
                                let _ = DrawFocusRect(item.hDC, &item.rcItem);
                            }
                            return LRESULT(1);
                        }
                    }
                    if lp.0 != 0 {
                        paint_button(&*(lp.0 as *const DRAWITEMSTRUCT), form);
                        return LRESULT(1);
                    }
                }
                WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                    let dc = HDC(wp.0 as *mut _);
                    SetTextColor(
                        dc,
                        rgb(
                            if form.task.is_some()
                                && (200..=202).contains(&GetDlgCtrlID(HWND(lp.0 as *mut _)))
                            {
                                theme::ERROR
                            } else {
                                WHITE
                            },
                        ),
                    );
                    SetBkMode(dc, TRANSPARENT);
                    SetBkColor(dc, rgb(if form.categorized { CARD } else { BG }));
                    return LRESULT(if form.categorized {
                        form.skin.card.0
                    } else {
                        form.skin.background.0
                    } as isize);
                }
                WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                    let dc = HDC(wp.0 as *mut _);
                    SetTextColor(dc, rgb(WHITE));
                    SetBkColor(dc, rgb(INPUT));
                    return LRESULT(form.skin.input.0 as isize);
                }
                WM_CLOSE => {
                    form.done.set(true);
                    return LRESULT(0);
                }
                WM_TIMER if wp.0 == 71 => {
                    let mut finished = None;
                    if let Some(download) = form.download.borrow().as_ref() {
                        while let Ok(event) = download.rx.try_recv() {
                            match event {
                                DownloadEvent::Progress(n, total) => {
                                    if *form.package_phase.borrow() != PackagePhase::Canceling {
                                        *form.package_phase.borrow_mut() =
                                            if n >= total && total > 0 {
                                                PackagePhase::Installing
                                            } else {
                                                PackagePhase::Downloading(n, total)
                                            };
                                    }
                                }
                                DownloadEvent::Done(result) => {
                                    finished = Some((
                                        result,
                                        download.cancel.load(std::sync::atomic::Ordering::Relaxed),
                                    ))
                                }
                            }
                        }
                    }
                    if let Some((result, canceled)) = finished {
                        form.download.borrow_mut().take();
                        let _ = KillTimer(Some(hwnd), 71);
                        *form.package_phase.borrow_mut() = match result {
                            Ok(()) => {
                                SendMessageW(
                                    form.entries.borrow()[23].control,
                                    BM_SETCHECK,
                                    Some(WPARAM(1)),
                                    None,
                                );
                                PackagePhase::Ready
                            }
                            Err(_) if canceled => PackagePhase::Canceled,
                            Err(_) => PackagePhase::Failed,
                        };
                    }
                    refresh_package(hwnd, form);
                    return LRESULT(0);
                }
                WM_COMMAND => {
                    let command = wp.0 & 0xffff;
                    let notification = (wp.0 >> 16) as u32;
                    if (100..123).contains(&command)
                        && (notification == EN_CHANGE || notification == CBN_SELCHANGE)
                    {
                        let index = command - 100;
                        let clear_error =
                            form.validation_error
                                .borrow()
                                .as_ref()
                                .is_some_and(|(i, _)| {
                                    *i == index
                                        || ((*i == 9 || *i == 8) && (index == 8 || index == 9))
                                });
                        if clear_error {
                            *form.validation_error.borrow_mut() = None;
                            let _ = InvalidateRect(Some(hwnd), None, true);
                        }
                    }
                    if (500..500 + SECTION_COUNT).contains(&command) {
                        *form.validation_error.borrow_mut() = None;
                        show_section(hwnd, form, command - 500);
                        return LRESULT(0);
                    }
                    if command == 124 && form.categorized {
                        if let Some(d) = form.download.borrow().as_ref() {
                            d.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            *form.package_phase.borrow_mut() = PackagePhase::Canceling;
                            refresh_package(hwnd, form);
                            return LRESULT(0);
                        }
                        if crate::offline_translation::installed() {
                            SendMessageW(
                                form.entries.borrow()[23].control,
                                BM_SETCHECK,
                                Some(WPARAM(1)),
                                None,
                            );
                        } else {
                            *form.package_phase.borrow_mut() = PackagePhase::Connecting;
                            *form.download.borrow_mut() = Some(Download::start());
                            refresh_package(hwnd, form);
                            SetTimer(Some(hwnd), 71, 150, None);
                        }
                        return LRESULT(0);
                    }
                    match command {
                        1 => {
                            if form.categorized
                                && SendMessageW(
                                    form.entries.borrow()[23].control,
                                    BM_GETCHECK,
                                    None,
                                    None,
                                )
                                .0 == 1
                                && !crate::offline_translation::installed()
                            {
                                let _ = MessageBoxW(
                                    Some(hwnd),
                                    &windows::core::HSTRING::from(if form.english {
                                        "Download the package before enabling offline translation."
                                    } else {
                                        "Baixe o pacote antes de ativar a tradução offline."
                                    }),
                                    w!("Nivumi"),
                                    MB_OK | MB_ICONINFORMATION,
                                );
                                return LRESULT(0);
                            }
                            let mut values = Vec::new();
                            let entries = form.entries.borrow().clone();
                            for (i, entry) in entries.iter().enumerate() {
                                let value = match &entry.kind {
                                    Kind::Check => {
                                        if SendMessageW(entry.control, BM_GETCHECK, None, None).0
                                            == 1
                                        {
                                            "1".into()
                                        } else {
                                            "0".into()
                                        }
                                    }
                                    Kind::Choice(_) => {
                                        SendMessageW(entry.control, CB_GETCURSEL, None, None)
                                            .0
                                            .max(0)
                                            .to_string()
                                    }
                                    _ => {
                                        let mut buf = vec![
                                            0;
                                            GetWindowTextLengthW(entry.control).max(0)
                                                as usize
                                                + 1
                                        ];
                                        let len = GetWindowTextW(entry.control, &mut buf);
                                        let text = String::from_utf16_lossy(&buf[..len as usize]);
                                        let text = if matches!(entry.kind, Kind::Number(_, _)) {
                                            text.replace(',', ".")
                                        } else {
                                            text
                                        };
                                        if matches!(entry.kind, Kind::Multiline) {
                                            text
                                        } else {
                                            text.trim().into()
                                        }
                                    }
                                };
                                if let Kind::Number(min, max) = entry.kind {
                                    if !value.parse::<f64>().is_ok_and(|n| {
                                        n.is_finite()
                                            && n >= min
                                            && n <= max
                                            && (min.fract() != 0.
                                                || max.fract() != 0.
                                                || n.fract() == 0.)
                                    }) {
                                        if form.categorized {
                                            show_section(hwnd, form, category(i));
                                        }
                                        *form.validation_error.borrow_mut() = Some((
                                            i,
                                            format!(
                                                "{}: {} {} – {}",
                                                entry.label,
                                                if form.english {
                                                    "enter a value between"
                                                } else {
                                                    "informe um valor entre"
                                                },
                                                min,
                                                max
                                            ),
                                        ));
                                        let _ = InvalidateRect(Some(hwnd), None, true);
                                        let _ = SetFocus(Some(entry.control));
                                        return LRESULT(0);
                                    }
                                }
                                values.push(value);
                            }
                            if entries.len() > 9
                                && values
                                    .get(8)
                                    .and_then(|v| v.parse::<usize>().ok())
                                    .zip(values.get(9).and_then(|v| v.parse::<usize>().ok()))
                                    .is_some_and(|(start, end)| start > end)
                            {
                                let index = 9;
                                let error = if form.english {
                                    "End time must be later than start time."
                                } else {
                                    "O fim deve ser posterior ao início."
                                };
                                *form.validation_error.borrow_mut() = Some((index, error.into()));
                                if form.categorized {
                                    show_section(hwnd, form, category(index));
                                }
                                let _ = InvalidateRect(Some(hwnd), None, true);
                                let _ = SetFocus(Some(entries[index].control));
                                return LRESULT(0);
                            }
                            *form.validation_error.borrow_mut() = None;
                            for (entry, value) in form.entries.borrow_mut().iter_mut().zip(values) {
                                entry.value = value;
                            }
                            form.accepted.set(true);
                            form.done.set(true);
                            return LRESULT(0);
                        }
                        2 => {
                            form.done.set(true);
                            return LRESULT(0);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}
// This private helper mirrors CreateWindowExW geometry arguments.
#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn field_procedure(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(field_procedure), id);
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        if id == 2 && msg == WM_PAINT {
            if let Ok(parent) = GetParent(hwnd) {
                let ptr = GetWindowLongPtrW(parent, GWLP_USERDATA) as *const Form;
                if !ptr.is_null() {
                    let form = &*ptr;
                    let dc = GetDC(Some(hwnd));
                    let mut rect = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rect);
                    FillRect(dc, &rect, form.skin.card);
                    let focused = windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() == hwnd;
                    round(dc, rect, if focused { GREEN } else { BORDER }, 14);
                    round(
                        dc,
                        RECT {
                            left: 1,
                            top: 1,
                            right: rect.right - 1,
                            bottom: rect.bottom - 1,
                        },
                        INPUT,
                        12,
                    );
                    let mut text = vec![0; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
                    let n = GetWindowTextW(hwnd, &mut text);
                    text_draw(
                        dc,
                        &String::from_utf16_lossy(&text[..n as usize]),
                        RECT {
                            left: 10,
                            top: 0,
                            right: rect.right - 26,
                            bottom: rect.bottom,
                        },
                        WHITE,
                        form.skin.font,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                    );
                    text_draw(
                        dc,
                        "⌄",
                        RECT {
                            left: rect.right - 25,
                            top: 0,
                            right: rect.right - 5,
                            bottom: rect.bottom,
                        },
                        MUTED,
                        form.skin.font,
                        DT_SINGLELINE | DT_VCENTER | DT_CENTER,
                    );
                    ReleaseDC(Some(hwnd), dc);
                }
            }
        }
        if matches!(msg, WM_SETFOCUS | WM_KILLFOCUS) {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = InvalidateRect(Some(parent), None, false);
            }
        }
        result
    }
}
#[allow(clippy::too_many_arguments)]
unsafe fn child(
    parent: HWND,
    class: &str,
    text: &str,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: usize,
) -> windows::core::Result<HWND> {
    let class = wide(class);
    let text = wide(text);
    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(text.as_ptr()),
            WS_CHILD | WS_VISIBLE | style,
            x,
            y,
            w,
            h,
            Some(parent),
            Some(HMENU(id as *mut _)),
            Some(GetModuleHandleW(None)?.into()),
            None,
        )?;
        let ptr = GetWindowLongPtrW(parent, GWLP_USERDATA) as *const Form;
        if !ptr.is_null() {
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM((&*ptr).skin.font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        let _ = SetWindowTheme(hwnd, w!("DarkMode_Explorer"), PCWSTR::null());
        if class == wide("COMBOBOX") {
            let _ = SetWindowSubclass(hwnd, Some(field_procedure), 2, 0);
        }
        if class == wide("EDIT") {
            let _ = SetWindowSubclass(hwnd, Some(field_procedure), 1, 0);
        }
        Ok(hwnd)
    }
}
fn form(owner: HWND, title: &str, entries: Vec<Entry>, english: bool) -> Option<Vec<String>> {
    form_at(owner, title, entries, english, 0)
}
fn form_at(
    owner: HWND,
    title: &str,
    entries: Vec<Entry>,
    english: bool,
    initial_section: usize,
) -> Option<Vec<String>> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = w!("Nivumi.NativeForm.Dark");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let categorized = entries.len() > 12;
        let columns = 1;
        let rows = entries.len().div_ceil(columns);
        let width = if categorized { 810 } else { 480 };
        let multiline = entries
            .iter()
            .any(|entry| matches!(entry.kind, Kind::Multiline));
        let desired_height = if categorized {
            690
        } else if multiline {
            330
        } else {
            rows as i32 * 72 + 135
        };
        let height = desired_height.min(GetSystemMetrics(SM_CYSCREEN) - 30);
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let x = ((screen_w - width) / 2).max(0);
        let y = ((screen_h - height) / 2).max(0);
        let title = wide(title);
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class,
            PCWSTR(title.as_ptr()),
            WS_CAPTION | WS_SYSMENU | WS_POPUP | WS_CLIPCHILDREN,
            x,
            y,
            width,
            height,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        crate::brand::apply_window(hwnd).ok()?;
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            std::mem::size_of_val(&corner) as u32,
        );
        for (attribute, color) in [
            (DWMWA_CAPTION_COLOR, BG),
            (DWMWA_BORDER_COLOR, BORDER),
            (DWMWA_TEXT_COLOR, WHITE),
        ] {
            let color = rgb(color);
            let _ = DwmSetWindowAttribute(
                hwnd,
                attribute,
                &color as *const _ as *const _,
                std::mem::size_of_val(&color) as u32,
            );
        }
        let state = Box::new(Form {
            package_phase: RefCell::new(if crate::offline_translation::installed() {
                PackagePhase::Ready
            } else {
                PackagePhase::Available
            }),
            download: RefCell::new(None),
            entries: RefCell::new(entries),
            done: Cell::new(false),
            accepted: Cell::new(false),
            english,
            categorized,
            section: Cell::new(0),
            rows: RefCell::new(Vec::new()),
            sidebar_buttons: RefCell::new(Vec::new()),
            hovered_btn: Cell::new(None),
            skin: Skin::new(),
            task: None,
            validation_error: RefCell::new(None),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&*state as *const Form) as isize);
        let dark = BOOL(1);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const BOOL).cast(),
            std::mem::size_of::<BOOL>() as u32,
        );
        let mut bounds = RECT::default();
        let _ = GetClientRect(hwnd, &mut bounds);
        let setup = (|| -> windows::core::Result<()> {
            if categorized {
                for (i, label) in sections(english).iter().enumerate() {
                    let btn = child(
                        hwnd,
                        "BUTTON",
                        label,
                        WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                        14,
                        101 + i as i32 * 36,
                        182,
                        32,
                        500 + i,
                    )?;
                    let _ = SetWindowSubclass(btn, Some(button_procedure), 500 + i, 0);
                    state.sidebar_buttons.borrow_mut().push(btn);
                }
            }
            let mut entries: Vec<_> = state
                .entries
                .borrow()
                .clone()
                .into_iter()
                .enumerate()
                .collect();
            state.rows.borrow_mut().resize(entries.len(), Vec::new());
            if categorized {
                entries.sort_by_key(|(i, _)| match i {
                    23 => 22,
                    24 => 23,
                    22 => 24,
                    _ => *i,
                });
            }
            let mut section_rows = [0; SECTION_COUNT];
            for (i, mut entry) in entries {
                let section = category(i);
                let row = if categorized {
                    let row = section_rows[section];
                    section_rows[section] += 1;
                    row
                } else {
                    i as i32
                };
                let x = if categorized { 230 } else { 24 };
                let y = if categorized {
                    if section == 8 {
                        match i {
                            22 => 410,
                            23 => 100,
                            _ => 204,
                        }
                    } else {
                        100 + row * theme::ROW_PITCH
                    }
                } else {
                    20 + row * 72
                };
                let mut controls = Vec::new();
                match &entry.kind {
                    Kind::Download => {
                        use windows::Win32::UI::Controls::{
                            InitCommonControlsEx, ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX,
                        };
                        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                            dwICC: ICC_PROGRESS_CLASS,
                        });
                        controls.push(child(
                            hwnd,
                            "STATIC",
                            if english {
                                "English → Brazilian Portuguese"
                            } else {
                                "Inglês → português brasileiro"
                            },
                            WINDOW_STYLE(0),
                            x,
                            y + 14,
                            520,
                            24,
                            0,
                        )?);
                        controls.push(child(
                            hwnd,
                            "STATIC",
                            if english {
                                "Automatic translation when no human translation is available."
                            } else {
                                "Tradução automática quando não há tradução humana."
                            },
                            WINDOW_STYLE(0),
                            x,
                            y + 40,
                            520,
                            22,
                            0,
                        )?);
                        controls.push(child(
                            hwnd,
                            "STATIC",
                            "",
                            WINDOW_STYLE(0),
                            x,
                            y + 70,
                            520,
                            24,
                            4000,
                        )?);
                        let progress = child(
                            hwnd,
                            "msctls_progress32",
                            "",
                            WINDOW_STYLE(windows::Win32::UI::Controls::PBS_SMOOTH),
                            x,
                            y + 100,
                            520,
                            10,
                            4001,
                        )?;
                        let _ = SetWindowTheme(progress, w!(""), w!(""));
                        controls.push(progress);
                        entry.control = child(
                            hwnd,
                            "BUTTON",
                            "",
                            WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                            x,
                            y + 124,
                            220,
                            38,
                            100 + i,
                        )?;
                        let _ = SetWindowSubclass(entry.control, Some(button_procedure), 125, 0);
                        controls.push(child(
                            hwnd,
                            "STATIC",
                            "",
                            WINDOW_STYLE(0),
                            x,
                            y + 168,
                            520,
                            30,
                            4002,
                        )?);
                    }
                    Kind::Check => {
                        entry.control = child(
                            hwnd,
                            "BUTTON",
                            &entry.label,
                            WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                            if categorized { 220 } else { x },
                            y + 4,
                            if categorized { 544 } else { 418 },
                            50,
                            100 + i,
                        )?;
                        SendMessageW(
                            entry.control,
                            BM_SETCHECK,
                            Some(WPARAM(usize::from(entry.value == "1"))),
                            None,
                        );
                        let _ = SetWindowSubclass(entry.control, Some(toggle_procedure), 1, 0);
                    }
                    _ => {
                        let label = child(
                            hwnd,
                            "STATIC",
                            &entry.label,
                            WINDOW_STYLE(0),
                            x,
                            y + 8,
                            if categorized {
                                if matches!(entry.kind, Kind::Multiline) {
                                    520
                                } else {
                                    330
                                }
                            } else {
                                418
                            },
                            22,
                            0,
                        )?;
                        controls.push(label);
                        let wide_multiline = categorized && matches!(entry.kind, Kind::Multiline);
                        let input_x = if wide_multiline {
                            x
                        } else if categorized {
                            590
                        } else {
                            x
                        };
                        let input_y = if wide_multiline {
                            y + 66
                        } else if categorized {
                            y + 15
                        } else {
                            y + 33
                        };
                        let input_width = if wide_multiline {
                            520
                        } else if categorized {
                            160
                        } else {
                            418
                        };
                        match &entry.kind {
                            Kind::Choice(options) => {
                                entry.control = child(
                                    hwnd,
                                    "COMBOBOX",
                                    "",
                                    WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
                                    input_x,
                                    input_y,
                                    input_width,
                                    180,
                                    100 + i,
                                )?;
                                for option in options {
                                    let t = wide(option);
                                    SendMessageW(
                                        entry.control,
                                        CB_ADDSTRING,
                                        None,
                                        Some(LPARAM(t.as_ptr() as isize)),
                                    );
                                }
                                SendMessageW(
                                    entry.control,
                                    CB_SETCURSEL,
                                    Some(WPARAM(entry.value.parse().unwrap_or(0))),
                                    None,
                                );
                            }
                            _ => {
                                let multi = matches!(entry.kind, Kind::Multiline);
                                let style = if multi {
                                    WS_VSCROLL
                                        | WINDOW_STYLE(
                                            (ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32,
                                        )
                                } else {
                                    WINDOW_STYLE(
                                        ES_AUTOHSCROLL as u32
                                            | if matches!(entry.kind, Kind::Number(min, max) if min.fract() == 0. && max.fract() == 0.)
                                            {
                                                ES_NUMBER as u32
                                            } else {
                                                0
                                            },
                                    )
                                };
                                entry.control = child(
                                    hwnd,
                                    "EDIT",
                                    &entry.value,
                                    WS_TABSTOP | style,
                                    input_x,
                                    input_y,
                                    input_width,
                                    if wide_multiline {
                                        if section == 8 {
                                            110
                                        } else {
                                            230
                                        }
                                    } else if multi {
                                        170
                                    } else {
                                        27
                                    },
                                    100 + i,
                                )?;
                            }
                        }
                    }
                }
                controls.push(entry.control);
                state.rows.borrow_mut()[i] = controls;
                state.entries.borrow_mut()[i] = entry;
            }
            let save_btn = child(
                hwnd,
                "BUTTON",
                if categorized {
                    if english {
                        "Save changes"
                    } else {
                        "Salvar alterações"
                    }
                } else if english {
                    "Save"
                } else {
                    "Salvar"
                },
                WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                bounds.right - if categorized { 256 } else { 220 },
                bounds.bottom - 49,
                if categorized { 130 } else { 94 },
                31,
                1,
            )?;
            let _ = SetWindowSubclass(save_btn, Some(button_procedure), 1, 0);
            let cancel_btn = child(
                hwnd,
                "BUTTON",
                if english { "Cancel" } else { "Cancelar" },
                WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                bounds.right - 116,
                bounds.bottom - 49,
                94,
                31,
                2,
            )?;
            let _ = SetWindowSubclass(cancel_btn, Some(button_procedure), 2, 0);
            Ok(())
        })();
        if setup.is_err() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(hwnd);
            return None;
        }
        if categorized {
            show_section(hwnd, &state, initial_section.min(SECTION_COUNT - 1));
            refresh_package(hwnd, &state);
        }
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let first = state
            .entries
            .borrow()
            .iter()
            .enumerate()
            .find(|(i, _)| !categorized || category(*i) == initial_section.min(SECTION_COUNT - 1))
            .map(|(_, entry)| entry.control);
        if let Some(control) = first {
            let _ = SetFocus(Some(control));
        }
        let mut msg = MSG::default();
        loop {
            if state.done.get() {
                break;
            }
            let result = GetMessageW(&mut msg, None, 0, 0).0;
            if result <= 0 {
                if result == 0 {
                    PostQuitMessage(msg.wParam.0 as i32);
                }
                break;
            }
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let _ = DestroyWindow(hwnd);
        let _ = EnableWindow(owner, true);
        let _ = SetForegroundWindow(owner);
        state.accepted.get().then(|| {
            state
                .entries
                .borrow()
                .iter()
                .map(|e| e.value.clone())
                .collect()
        })
    }
}
/// Edits a copy; Cancel leaves all caller settings unchanged.
pub fn settings(owner: HWND, s: &mut Settings) -> bool {
    settings_at(owner, s, 0)
}
pub fn hydration_settings(owner: HWND, s: &mut Settings) -> bool {
    settings_at(owner, s, 5)
}
fn settings_entries(s: &Settings) -> Vec<Entry> {
    let en = s.language == "en";
    let l = |pt: &str, en_text: &str| {
        if en {
            en_text.to_string()
        } else {
            pt.to_string()
        }
    };
    vec![
        Entry::choice(
            &l("Idioma", "Language"),
            usize::from(en),
            &["Português (Brasil)", "English"],
        ),
        Entry::number(
            &l("Monitor (0 = principal)", "Monitor (0 = primary)"),
            s.monitor,
            0.,
            32.,
        ),
        Entry::number(
            &l("Tamanho da interface", "Interface size"),
            s.scale,
            0.75,
            2.,
        ),
        Entry::number(
            &l("Foco: minutos", "Focus: minutes"),
            s.focus_minutes,
            1.,
            240.,
        ),
        Entry::number(
            &l("Pausa curta: minutos", "Short break: minutes"),
            s.short_break_minutes,
            1.,
            120.,
        ),
        Entry::number(
            &l("Pausa longa: minutos", "Long break: minutes"),
            s.long_break_minutes,
            1.,
            180.,
        ),
        Entry::number(
            &l("Meta de água: ml", "Water goal: ml"),
            s.water_goal_ml,
            100.,
            10000.,
        ),
        Entry::number(
            &l("Recipiente: ml", "Bottle: ml"),
            s.water_bottle_ml,
            25.,
            3000.,
        ),
        Entry::choice_owned(
            &l("Início dos lembretes", "Reminders start"),
            s.water_start_hour as usize,
            (0..24).map(|hour| format!("{hour:02}:00")).collect(),
        ),
        Entry::choice_owned(
            &l("Fim dos lembretes", "Reminders end"),
            s.water_end_hour.saturating_sub(1) as usize,
            (1..=24).map(|hour| format!("{hour:02}:00")).collect(),
        ),
        Entry::check(
            &l(
                "Abrir com clique (em vez de hover)",
                "Open on click (instead of hover)",
            ),
            s.open_click,
        ),
        Entry::check(&l("Notch mínima", "Minimal notch"), s.minimal),
        Entry::check(
            &l(
                "Ocultar em tela cheia ou janela maximizada",
                "Hide in full screen or maximized windows",
            ),
            s.hide_fullscreen,
        ),
        Entry::check(&l("Reduzir animações", "Reduce motion"), s.reduced_motion),
        Entry::check(&l("Sons de lembretes", "Reminder sounds"), s.sounds),
        Entry::check(&l("Iniciar com o Windows", "Start with Windows"), s.startup),
        Entry::check(
            &l("Semana começa na segunda-feira", "Week starts Monday"),
            s.week_monday,
        ),
        Entry::check(
            &l("Limpar Prateleira ao iniciar", "Clear Shelf at startup"),
            s.shelf_clear,
        ),
        Entry::check(
            &l("Indicador de bateria", "Battery indicator"),
            s.show_battery,
        ),
        Entry::check(&l("Avisos de bateria", "Battery popups"), s.popup_battery),
        Entry::check(
            &l("Avisos de conectividade", "Connectivity popups"),
            s.popup_wifi,
        ),
        Entry::check(
            &l("Avisos de Bluetooth", "Bluetooth popups"),
            s.popup_bluetooth,
        ),
        Entry {
            kind: Kind::Multiline,
            ..Entry::text(
                &l(
                    "Termos extras ignorados na busca",
                    "Extra terms ignored in search",
                ),
                s.lyrics.ignored_title_terms.join("\r\n"),
            )
        },
        Entry::check(
            &l("Usar tradução offline", "Use offline translation"),
            s.lyrics.offline_translation,
        ),
        Entry {
            kind: Kind::Download,
            ..Entry::text(&l("Baixar e ativar", "Download and enable"), "")
        },
        Entry::choice(
            &l("Animação ao trocar verso", "Verse focus animation"),
            s.lyrics.focus_animation,
            &if en {
                ["No animation", "Fade", "Slide", "Soft highlight"]
            } else {
                ["Sem animação", "Dissolver", "Deslizar", "Realce suave"]
            },
        ),
        Entry::check(
            &l("Tradução em todos os versos", "Translation on all verses"),
            s.lyrics.translation_all_verses,
        ),
    ]
}
fn settings_at(owner: HWND, s: &mut Settings, initial_section: usize) -> bool {
    let en = s.en();
    let entries = settings_entries(s);
    let l = |pt: &str, en_text: &str| {
        if en {
            en_text.to_owned()
        } else {
            pt.to_owned()
        }
    };
    let Some(v) = form_at(
        owner,
        &l("Nivumi — Configurações", "Nivumi — Settings"),
        entries,
        en,
        initial_section,
    ) else {
        return false;
    };
    apply_settings_values(s, &v)
}
fn apply_settings_values(s: &mut Settings, v: &[String]) -> bool {
    if v.len() != 27 {
        return false;
    }
    let mut copy = s.clone();
    copy.language = if v[0] == "1" { "en" } else { "pt-BR" }.into();
    copy.monitor = v[1].parse().unwrap_or(0);
    copy.scale = v[2].parse().unwrap_or(1.);
    copy.focus_minutes = v[3].parse().unwrap_or(25);
    copy.short_break_minutes = v[4].parse().unwrap_or(5);
    copy.long_break_minutes = v[5].parse().unwrap_or(15);
    copy.water_goal_ml = v[6].parse().unwrap_or(1500);
    copy.water_bottle_ml = v[7].parse().unwrap_or(250);
    copy.water_start_hour = v[8].parse().unwrap_or(8);
    copy.water_end_hour = v[9].parse::<u32>().unwrap_or(21) + 1;
    copy.open_click = v[10] == "1";
    copy.minimal = v[11] == "1";
    copy.hide_fullscreen = v[12] == "1";
    copy.reduced_motion = v[13] == "1";
    copy.sounds = v[14] == "1";
    copy.startup = v[15] == "1";
    copy.week_monday = v[16] == "1";
    copy.calendar_week = false;
    copy.shelf_clear = v[17] == "1";
    copy.show_battery = v[18] == "1";
    copy.popup_battery = v[19] == "1";
    copy.popup_wifi = v[20] == "1";
    copy.popup_bluetooth = v[21] == "1";
    copy.lyrics.focus_animation = v[25].parse().unwrap_or(1);
    copy.lyrics.translation_all_verses = v[26] == "1";
    copy.lyrics.offline_translation = v[23] == "1" && crate::offline_translation::installed();
    copy.lyrics.ignored_title_terms = crate::letras::normalize_ignored_terms(v[22].lines());
    copy.normalize();
    *s = copy;
    true
}

pub fn text(owner: HWND, en: bool) -> Option<String> {
    let l = |pt: &'static str, en_text: &'static str| -> &'static str {
        if en {
            en_text
        } else {
            pt
        }
    };
    form(
        owner,
        l("Nivumi — Texto", "Nivumi — Text"),
        vec![Entry {
            kind: Kind::Multiline,
            ..Entry::text(l("Texto ou link", "Text or link"), "")
        }],
        en,
    )
    .and_then(|v| (!v[0].trim().is_empty()).then(|| v[0].clone()))
}

mod task_form;
pub use task_form::task;
use task_form::{task_command, TaskForm};

#[cfg(test)]
mod settings_navigation_tests {
    use super::*;
    #[test]
    fn settings_sections_have_controls_and_localized_descriptions() {
        let mut counts = [0; SECTION_COUNT];
        for i in 0..28 {
            counts[category(i)] += 1;
        }
        assert!(counts[..SECTION_COUNT - 1].iter().all(|count| *count > 0));
        assert_eq!(counts[SECTION_COUNT - 1], 0);
        for en in [false, true] {
            assert!(sections(en).iter().all(|label| !label.is_empty()));
            for i in 0..SECTION_COUNT {
                assert!(!subtitle(i, en).is_empty());
            }
        }
        assert_eq!(sections(false)[6], "Prateleira");
        assert_eq!(sections(true)[6], "Shelf");
        assert_eq!(sections(false)[0], "Geral");
        assert_eq!(sections(true)[0], "General");
        assert_eq!(sections(false)[9], "Leitura de letras");
        assert_eq!(sections(false)[10], "Sobre");
        assert_eq!(sections(true)[9], "Lyrics display");
        assert_eq!(sections(true)[10], "About");
    }
}

#[cfg(test)]
mod package_feedback_tests {
    use super::*;
    #[test]
    fn progress_is_bounded_and_never_divides_by_zero() {
        assert_eq!(
            package_ui(&PackagePhase::Downloading(10, 0), false).percent,
            0
        );
        assert_eq!(
            package_ui(&PackagePhase::Downloading(u64::MAX, 10), false).percent,
            100
        );
        assert_eq!(
            package_ui(&PackagePhase::Downloading(25, 100), true).percent,
            25
        );
    }
    #[test]
    fn cancellation_is_busy_until_worker_finishes() {
        for en in [false, true] {
            let canceling = package_ui(&PackagePhase::Canceling, en);
            assert!(canceling.busy);
            let canceled = package_ui(&PackagePhase::Canceled, en);
            assert!(!canceled.busy);
            assert_ne!(canceling.action, canceled.action);
            assert_ne!(canceling.status, canceled.status);
        }
    }
    #[test]
    fn finishing_download_is_not_the_same_as_ready() {
        for en in [false, true] {
            let installing = package_ui(&PackagePhase::Installing, en);
            let ready = package_ui(&PackagePhase::Ready, en);
            assert_eq!(installing.percent, ready.percent);
            assert!(installing.busy);
            assert!(!ready.busy);
            assert_ne!(installing.status, ready.status);
            assert!(!package_ui(&PackagePhase::Failed, en).busy);
        }
    }
}

#[cfg(test)]
mod branding_settings_tests {
    use super::*;
    #[test]
    fn settings_roundtrip_after_removing_mascot_keeps_fields_aligned() {
        let expected = Settings {
            language: "en".into(),
            monitor: 2,
            scale: 1.5,
            startup: true,
            minimal: true,
            reduced_motion: true,
            hide_fullscreen: false,
            sounds: false,
            week_monday: false,
            shelf_clear: true,
            water_start_hour: 6,
            water_end_hour: 23,
            ..Settings::default()
        };
        let entries = settings_entries(&expected);
        assert_eq!(entries.len(), 27);
        assert!(entries.iter().all(|e| !e.label.contains("Mascot")));
        assert_eq!(category(23), 8);
        assert_eq!(category(24), 8);
        assert_eq!(category(25), 9);
        let values: Vec<_> = entries.into_iter().map(|e| e.value).collect();
        let mut actual = Settings::default();
        assert!(apply_settings_values(&mut actual, &values));
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}
