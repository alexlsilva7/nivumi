//! Development-only render verification. Synthetic content never reaches production.
use super::*;
pub unsafe fn capture(app: &mut App, directory: PathBuf) -> Result<()> {
    if std::env::var_os("NIVUMI_DATA_DIR").is_none() {
        return Err(Error::new(
            E_FAIL,
            "Visual verification requires isolated NIVUMI_DATA_DIR",
        ));
    }
    std::fs::create_dir_all(&directory).map_err(|e| Error::new(E_FAIL, e.to_string()))?;
    app.state.settings.reduced_motion = true;
    app.state.settings.hide_fullscreen = false;
    app.state.apply(
        AppCommand::AddTask {
            title: "Verificar identidade Nivumi".into(),
            due_at: None,
        },
        Local::now(),
    );
    app.state.apply(AppCommand::RecordDrink, Local::now());
    for scale in [1., 1.5, 2.] {
        app.state.settings.scale = scale;
        app.media = MediaSnapshot::default();
        app.external_lyrics = None;
        app.accent = [Spring::new(0.); 3];
        app.page = 0;
        app.expanded = false;
        app.expansion.snap(0.);
        app.navigation.snap(0.);
        app.resize();
        let (w, h) = app.target_size();
        app.body_width.snap(w);
        app.body_height.snap(h);
        nivumi::render::frame_capture::request(
            directory.join(format!("{}-compact.bmp", (scale * 100.) as u32)),
        );
        app.paint();
        for page in [0, 2, 3, 4, 5, 6] {
            capture_page(app, &directory, scale, page, "empty");
        }
        app.media = MediaSnapshot {
            title: "Verificação visual — dados sintéticos".into(),
            artist: "Nivumi".into(),
            duration: 210.,
            position: 65.,
            playing: true,
            accent: Some([0.24, 0.16, 0.36]),
            cover: include_bytes!("../assets/brand/icon-256.png").to_vec(),
            ..MediaSnapshot::default()
        };
        app.accent = [Spring::new(0.24), Spring::new(0.16), Spring::new(0.36)];
        app.lyric_lines = lyrics::parse_lrc("[00:00.00]Interface de verificação\n[00:30.00]Seu dia, no seu ritmo.\n[01:20.00]Prévia com conteúdo sintético");
        for line in &mut app.lyric_lines {
            line.translation = Some("Tradução de demonstração".into());
        }
        app.external_lyrics = Some(ExternalLyrics {
            original:
                "Interface de verificação\nSeu dia, no seu ritmo.\nPrévia com conteúdo sintético"
                    .into(),
            translation: Some(
                "Interface de teste\nTradução de demonstração\nDados sintéticos".into(),
            ),
            ..ExternalLyrics::default()
        });
        app.lyrics_view_mode = 1;
        capture_page(app, &directory, scale, 0, "media");
        capture_page(app, &directory, scale, 1, "lyrics");
    }
    Ok(())
}
unsafe fn capture_page(
    app: &mut App,
    directory: &std::path::Path,
    scale: f32,
    page: usize,
    kind: &str,
) {
    app.page = page;
    app.expanded = true;
    app.expansion.snap(1.);
    app.player_layout.snap(if page == 1 { 1. } else { 0. });
    let (w, h) = app.target_size();
    app.body_width.snap(w);
    app.body_height.snap(h);
    app.resize();
    nivumi::render::frame_capture::request(
        directory.join(format!("{}-page-{page}-{kind}.bmp", (scale * 100.) as u32)),
    );
    app.paint();
}
