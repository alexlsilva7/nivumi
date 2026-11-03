fn main() {
    std::env::set_var(
        "NIVUMI_DATA_DIR",
        std::env::temp_dir().join(format!("nusa-settings-preview-{}", std::process::id())),
    );
    let mut settings = nivumi::model::Settings::default();
    nivumi::dialogs::settings(windows::Win32::Foundation::HWND::default(), &mut settings);
}
