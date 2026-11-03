//! Isolated synthetic translation probe. Set NUSA_REBUILD_DATA_DIR before running.
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if std::env::var_os("NIVUMI_DATA_DIR")
        .or_else(|| std::env::var_os("NUSA_REBUILD_DATA_DIR"))
        .is_none()
    {
        return Err("Set isolated NIVUMI_DATA_DIR".into());
    }
    let cancel = std::sync::atomic::AtomicBool::new(false);
    if std::env::args().any(|a| a == "--install") {
        nivumi::offline_translation::install(&cancel, |_, _| {})?;
    }
    let start = std::time::Instant::now();
    let mut engine = nivumi::offline_translation::Engine::load()?;
    let text="This is a beautiful morning.\n\nI am walking down the street with my friends.\nThis is a beautiful morning.";
    let result = engine.translate(text, &cancel)?;
    println!("{}\nElapsed: {:?}", result.translated, start.elapsed());
    assert_eq!(result.translated.lines().count(), 4);
    assert_eq!(
        result.translated.lines().next(),
        result.translated.lines().last()
    );
    assert!(!result.translated.contains("beautiful morning"));
    Ok(())
}
