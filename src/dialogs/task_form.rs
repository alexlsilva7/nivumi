//! Native task editor. Dates stay in local wall-clock time until Save.
use super::*;
use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use windows::Win32::UI::Controls::*;

pub(super) const COLORS: [u32; 8] = [
    theme::ACCENT,
    theme::POSITIVE,
    0xffd60a,
    0xff9f0a,
    0xff453a,
    0xbf5af2,
    0xff6482,
    0x64d2ff,
];
pub(super) struct TaskForm {
    old: crate::model::Task,
    result: Option<crate::model::Task>,
    title: HWND,
    date: HWND,
    time: HWND,
    reminder: HWND,
    color: HWND,
    hex: HWND,
    hex_label: HWND,
    errors: [HWND; 3],
}
fn label(en: bool, pt: &'static str, eng: &'static str) -> &'static str {
    if en {
        eng
    } else {
        pt
    }
}
unsafe fn set_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}
unsafe fn read_text(hwnd: HWND) -> String {
    unsafe {
        let mut buf = vec![0; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let len = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..len as usize])
            .trim()
            .to_string()
    }
}
fn parse_color(text: &str) -> Option<u32> {
    let text = text.trim().strip_prefix('#').unwrap_or(text.trim());
    (text.len() == 6 && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(text, 16).ok())
        .flatten()
}
fn local_timestamp(date: SYSTEMTIME, time: SYSTEMTIME) -> Option<i64> {
    NaiveDate::from_ymd_opt(date.wYear.into(), date.wMonth.into(), date.wDay.into())?
        .and_hms_opt(time.wHour.into(), time.wMinute.into(), 0)
        .and_then(|d| Local.from_local_datetime(&d).single())
        .map(|d| d.timestamp())
}
unsafe fn picker_value(hwnd: HWND) -> Option<SYSTEMTIME> {
    unsafe {
        let mut value = SYSTEMTIME::default();
        (SendMessageW(
            hwnd,
            DTM_GETSYSTEMTIME,
            None,
            Some(LPARAM((&mut value as *mut SYSTEMTIME) as isize)),
        )
        .0 == GDT_VALID.0 as isize)
            .then_some(value)
    }
}
unsafe fn set_date(hwnd: HWND, date: chrono::NaiveDate) {
    let value = SYSTEMTIME {
        wYear: date.year() as u16,
        wMonth: date.month() as u16,
        wDay: date.day() as u16,
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            hwnd,
            DTM_SETSYSTEMTIME,
            Some(WPARAM(GDT_VALID.0 as usize)),
            Some(LPARAM((&value as *const SYSTEMTIME) as isize)),
        );
    }
}
unsafe fn update_reminder(t: &TaskForm) {
    unsafe {
        let enabled = SendMessageW(t.reminder, CB_GETCURSEL, None, None).0 != 0;
        let _ = EnableWindow(t.date, enabled);
        let _ = EnableWindow(t.time, enabled);
        set_text(t.errors[1], "");
    }
}
pub(super) unsafe fn task_command(hwnd: HWND, form: &Form, t: &mut TaskForm, wp: WPARAM) {
    unsafe {
        let id = wp.0 & 0xffff;
        let notification = (wp.0 >> 16) as u32;
        let en = form.english;
        match id {
            2 => form.done.set(true),
            110 | 111 => {
                let today = Local::now().date_naive();
                if let Some(date) = if id == 111 {
                    today.succ_opt()
                } else {
                    Some(today)
                } {
                    set_date(t.date, date);
                    SendMessageW(t.reminder, CB_SETCURSEL, Some(WPARAM(1)), None);
                    update_reminder(t);
                }
            }
            103 if notification == CBN_SELCHANGE => update_reminder(t),
            104 if notification == CBN_SELCHANGE => {
                let advanced =
                    SendMessageW(t.color, CB_GETCURSEL, None, None).0 == COLORS.len() as isize;
                for control in [t.hex_label, t.hex] {
                    let _ = ShowWindow(control, if advanced { SW_SHOW } else { SW_HIDE });
                }
                set_text(t.errors[2], "");
            }
            1 => {
                for error in t.errors {
                    set_text(error, "");
                }
                let title = read_text(t.title);
                let reminder = SendMessageW(t.reminder, CB_GETCURSEL, None, None).0 != 0;
                let due = if reminder {
                    picker_value(t.date)
                        .zip(picker_value(t.time))
                        .and_then(|(d, h)| local_timestamp(d, h))
                        .map(Some)
                } else {
                    Some(None)
                };
                let index = SendMessageW(t.color, CB_GETCURSEL, None, None).0 as usize;
                let color = COLORS
                    .get(index)
                    .copied()
                    .or_else(|| parse_color(&read_text(t.hex)));
                let mut focus = None;
                if title.is_empty() {
                    set_text(
                        t.errors[0],
                        label(en, "Informe um título.", "Enter a title."),
                    );
                    focus = Some(t.title);
                }
                if due.is_none() {
                    set_text(
                        t.errors[1],
                        label(
                            en,
                            "Escolha uma data e hora local válidas, sem ambiguidade.",
                            "Choose a valid, unambiguous local date and time.",
                        ),
                    );
                    focus = focus.or(Some(t.date));
                }
                if color.is_none() {
                    set_text(
                        t.errors[2],
                        label(
                            en,
                            "Use 6 dígitos de 0 a 9 ou A a F (ex.: 7CA8FF).",
                            "Use 6 digits from 0–9 or A–F (e.g. 7CA8FF).",
                        ),
                    );
                    focus = focus.or(Some(t.hex));
                }
                if let Some(control) = focus {
                    let _ = SetFocus(Some(control));
                } else {
                    t.result = Some(crate::model::Task {
                        title,
                        due_at: due.unwrap(),
                        color: color.unwrap(),
                        notified: false,
                        ..t.old.clone()
                    });
                    form.done.set(true);
                }
            }
            100 if notification == EN_CHANGE => set_text(t.errors[0], ""),
            105 if notification == EN_CHANGE => set_text(t.errors[2], ""),
            _ => {}
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

pub fn task(
    owner: HWND,
    existing: Option<&crate::model::Task>,
    en: bool,
) -> Option<crate::model::Task> {
    unsafe {
        let old = existing.cloned().unwrap_or(crate::model::Task {
            id: 0,
            title: String::new(),
            due_at: None,
            done: false,
            color: COLORS[0],
            notified: false,
        });
        let initial = old
            .due_at
            .and_then(|s| Local.timestamp_opt(s, 0).single())
            .unwrap_or_else(Local::now);
        let instance = GetModuleHandleW(None).ok()?;
        let class = w!("Nivumi.TaskForm");
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_DATE_CLASSES,
        })
        .ok()
        .ok()?;
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        });
        let caption = wide(label(en, "Nivumi — Tarefa", "Nivumi — Task"));
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class,
            PCWSTR(caption.as_ptr()),
            WS_CAPTION | WS_SYSMENU | WS_POPUP | WS_CLIPCHILDREN,
            ((GetSystemMetrics(SM_CXSCREEN) - 520) / 2).max(0),
            ((GetSystemMetrics(SM_CYSCREEN) - 570) / 2).max(0),
            520,
            570,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        crate::brand::apply_window(hwnd).ok()?;
        let mut state = Box::new(Form {
            package_phase: RefCell::new(super::PackagePhase::Available),
            download: RefCell::new(None),
            entries: RefCell::new(vec![]),
            done: Cell::new(false),
            accepted: Cell::new(false),
            english: en,
            categorized: false,
            section: Cell::new(0),
            rows: RefCell::new(vec![]),
            sidebar_buttons: RefCell::new(vec![]),
            hovered_btn: Cell::new(None),
            skin: Skin::new(),
            task: None,
            validation_error: RefCell::new(None),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&*state as *const Form) as isize);
        let setup = (|| -> windows::core::Result<TaskForm> {
            let static_label = |text: &str, y, id| {
                child(hwnd, "STATIC", text, WINDOW_STYLE(0), 24, y, 460, 22, id)
            };
            static_label(label(en, "Título", "Title"), 20, 0)?;
            let title = child(
                hwnd,
                "EDIT",
                &old.title,
                WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                24,
                46,
                460,
                30,
                100,
            )?;
            let error_title = static_label("", 80, 200)?;
            static_label(label(en, "Lembrete", "Reminder"), 110, 0)?;
            let reminder = child(
                hwnd,
                "COMBOBOX",
                "",
                WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
                24,
                138,
                220,
                120,
                103,
            )?;
            for text in [
                label(en, "Sem lembrete", "No reminder"),
                label(en, "Escolher data e horário", "Choose date and time"),
            ] {
                let text = wide(text);
                SendMessageW(
                    reminder,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(text.as_ptr() as isize)),
                );
            }
            SendMessageW(
                reminder,
                CB_SETCURSEL,
                Some(WPARAM(usize::from(old.due_at.is_some()))),
                None,
            );
            child(
                hwnd,
                "BUTTON",
                label(en, "Hoje", "Today"),
                WS_TABSTOP,
                256,
                134,
                108,
                36,
                110,
            )?;
            child(
                hwnd,
                "BUTTON",
                label(en, "Amanhã", "Tomorrow"),
                WS_TABSTOP,
                376,
                134,
                108,
                36,
                111,
            )?;
            child(
                hwnd,
                "STATIC",
                label(en, "Data", "Date"),
                WINDOW_STYLE(0),
                24,
                180,
                220,
                22,
                0,
            )?;
            child(
                hwnd,
                "STATIC",
                label(en, "Horário", "Time"),
                WINDOW_STYLE(0),
                264,
                180,
                220,
                22,
                0,
            )?;
            let date = child(
                hwnd,
                "SysDateTimePick32",
                "",
                WS_TABSTOP | WINDOW_STYLE(DTS_SHORTDATEFORMAT),
                24,
                206,
                220,
                32,
                101,
            )?;
            let time = child(
                hwnd,
                "SysDateTimePick32",
                "",
                WS_TABSTOP | WINDOW_STYLE(DTS_TIMEFORMAT),
                264,
                206,
                220,
                32,
                102,
            )?;
            let format = wide("HH:mm");
            SendMessageW(
                time,
                DTM_SETFORMATW,
                None,
                Some(LPARAM(format.as_ptr() as isize)),
            );
            set_date(date, initial.date_naive());
            let value = SYSTEMTIME {
                wYear: initial.year() as u16,
                wMonth: initial.month() as u16,
                wDay: initial.day() as u16,
                wHour: initial.hour() as u16,
                wMinute: initial.minute() as u16,
                ..Default::default()
            };
            SendMessageW(
                time,
                DTM_SETSYSTEMTIME,
                Some(WPARAM(GDT_VALID.0 as usize)),
                Some(LPARAM((&value as *const SYSTEMTIME) as isize)),
            );
            let error_date = static_label("", 244, 201)?;
            static_label(label(en, "Cor", "Color"), 280, 0)?;
            let color = child(
                hwnd,
                "COMBOBOX",
                "",
                WS_TABSTOP
                    | WS_VSCROLL
                    | WINDOW_STYLE((CBS_DROPDOWNLIST | CBS_OWNERDRAWFIXED | CBS_HASSTRINGS) as u32),
                24,
                306,
                460,
                240,
                104,
            )?;
            let pt = [
                "Azul",
                "Verde",
                "Amarelo",
                "Laranja",
                "Vermelho",
                "Roxo",
                "Rosa",
                "Ciano",
                "Avançado: cor personalizada",
            ];
            let eng = [
                "Blue",
                "Green",
                "Yellow",
                "Orange",
                "Red",
                "Purple",
                "Pink",
                "Cyan",
                "Advanced: custom color",
            ];
            for text in if en { eng } else { pt } {
                let text = wide(text);
                SendMessageW(
                    color,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(text.as_ptr() as isize)),
                );
            }
            SendMessageW(color, CB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(28)));
            SendMessageW(
                color,
                CB_SETITEMHEIGHT,
                Some(WPARAM(usize::MAX)),
                Some(LPARAM(28)),
            );
            let selected = COLORS
                .iter()
                .position(|&c| c == old.color)
                .unwrap_or(COLORS.len());
            SendMessageW(color, CB_SETCURSEL, Some(WPARAM(selected)), None);
            let hex_label = static_label(
                label(
                    en,
                    "Cor personalizada (hexadecimal)",
                    "Custom color (hexadecimal)",
                ),
                346,
                0,
            )?;
            let hex = child(
                hwnd,
                "EDIT",
                &format!("{:06X}", old.color),
                WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                24,
                372,
                460,
                30,
                105,
            )?;
            for control in [hex_label, hex] {
                let _ = ShowWindow(
                    control,
                    if selected == COLORS.len() {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
            let error_color = static_label("", 408, 202)?;
            let save = child(
                hwnd,
                "BUTTON",
                label(en, "Salvar", "Save"),
                WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                264,
                472,
                104,
                36,
                1,
            )?;
            let _ = SetWindowSubclass(save, Some(button_procedure), 1, 0);
            let cancel = child(
                hwnd,
                "BUTTON",
                label(en, "Cancelar", "Cancel"),
                WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                380,
                472,
                104,
                36,
                2,
            )?;
            let _ = SetWindowSubclass(cancel, Some(button_procedure), 2, 0);
            Ok(TaskForm {
                old,
                result: None,
                title,
                date,
                time,
                reminder,
                color,
                hex,
                hex_label,
                errors: [error_title, error_date, error_color],
            })
        })();
        let Ok(task) = setup else {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(hwnd);
            return None;
        };
        update_reminder(&task);
        let first = task.title;
        state.task = Some(RefCell::new(task));
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(first));
        let mut msg = MSG::default();
        while !state.done.get() {
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
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        let _ = DestroyWindow(hwnd);
        let _ = EnableWindow(owner, true);
        let _ = SetForegroundWindow(owner);
        let result = state.task.as_ref()?.borrow_mut().result.take();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_colors_require_six_hex_digits() {
        assert_eq!(parse_color(" #7CA8ff "), Some(0x7ca8ff));
        for value in ["", "FFF", "1234567", "GG1234", "##123456"] {
            assert_eq!(parse_color(value), None);
        }
    }
    #[test]
    fn invalid_calendar_dates_are_rejected() {
        let date = SYSTEMTIME {
            wYear: 2026,
            wMonth: 2,
            wDay: 30,
            ..Default::default()
        };
        assert_eq!(local_timestamp(date, SYSTEMTIME::default()), None);
    }
}
