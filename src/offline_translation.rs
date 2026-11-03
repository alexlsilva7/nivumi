//! Optional, pinned CPU translation package. No network is used by inference.
use ort::{
    session::Session,
    value::{DynValue, Tensor},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
pub type Error = Box<dyn std::error::Error + Send + Sync>;
#[derive(Deserialize)]
struct Manifest {
    version: String,
    files: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    url: String,
    size: u64,
    sha256: String,
}
fn manifest() -> Manifest {
    serde_json::from_str(include_str!("../assets/offline-translation.json"))
        .expect("bundled manifest")
}
pub fn root() -> PathBuf {
    crate::storage::default_path()
        .parent()
        .unwrap()
        .join("models")
        .join(manifest().version)
}
pub fn installed() -> bool {
    root().join("ready.json").is_file()
        && root().join("onnxruntime.dll").is_file()
        && manifest()
            .files
            .iter()
            .filter(|a| a.name != "runtime.zip")
            .all(|a| fs::metadata(root().join(&a.name)).is_ok_and(|m| m.len() == a.size))
}
fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn verify(path: &Path, asset: &Asset) -> Result<(), Error> {
    let mut f = fs::File::open(path)?;
    if f.metadata()?.len() != asset.size {
        return Err("Package size mismatch".into());
    }
    let mut hash = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    if format!("{:x}", hash.finalize()) != asset.sha256 {
        return Err("Package SHA-256 mismatch".into());
    }
    Ok(())
}
pub fn install(cancel: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Download canceled".into());
    }
    let m = manifest();
    let dest = root();
    if installed() {
        return Ok(());
    }
    let stage = dest.with_extension("download");
    fs::create_dir_all(&stage)?;
    let total = m.files.iter().map(|a| a.size).sum();
    let mut done = 0;
    let agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_per_read(Some(Duration::from_secs(5)))
        .timeout_global(Some(Duration::from_secs(600)))
        .build()
        .new_agent();
    for a in &m.files {
        if cancel.load(Ordering::Relaxed) {
            return Err("Download canceled".into());
        }
        let path = stage.join(&a.name);
        if verify(&path, a).is_err() {
            let mut response = agent.get(&a.url).call()?;
            let mut reader = response.body_mut().as_reader();
            let mut file = fs::File::create(&path)?;
            let mut bytes = 0;
            let mut buf = [0; 65536];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Download canceled".into());
                }
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                bytes += n as u64;
                if bytes > a.size {
                    return Err("Download exceeds manifest size".into());
                }
                file.write_all(&buf[..n])?;
                progress(done + bytes, total);
            }
            file.sync_all()?;
            verify(&path, a)?;
        }
        done += a.size;
        progress(done, total);
    }
    let mut zip = zip::ZipArchive::new(fs::File::open(stage.join("runtime.zip"))?)?;
    let mut dll = zip.by_name("onnxruntime-win-x64-1.31.1/lib/onnxruntime.dll")?;
    let mut out = fs::File::create(stage.join("onnxruntime.dll"))?;
    std::io::copy(&mut dll, &mut out)?;
    out.sync_all()?;
    drop(out);
    drop(dll);
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().rsplit('/').next().unwrap_or("").to_owned();
        if ["LICENSE", "ThirdPartyNotices.txt"].contains(&name.as_str()) {
            let mut file = fs::File::create(stage.join(name))?;
            std::io::copy(&mut entry, &mut file)?;
        }
    }
    drop(zip);
    fs::remove_file(stage.join("runtime.zip"))?;
    fs::write(
        stage.join("MODEL-LICENSE.txt"),
        include_str!("../assets/offline-model-LICENSE.txt"),
    )?;
    fs::write(stage.join("MODEL-NOTICE.txt"),"OPUS-MT by the University of Helsinki Language Technology Research Group. ONNX conversion: Xenova. Source: https://huggingface.co/Helsinki-NLP/opus-mt-en-ROMANCE")?;
    fs::write(
        stage.join("ready.json"),
        include_str!("../assets/offline-translation.json"),
    )?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Download canceled".into());
    }
    if dest.exists() {
        fs::rename(
            &dest,
            dest.with_extension(format!(
                "invalid-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis()
            )),
        )?;
    }
    fs::rename(stage, dest)?;
    Ok(())
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum TranslationOrigin {
    #[default]
    Human,
    Offline,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Translation {
    pub original: String,
    pub translated: String,
}
pub fn cache_key(song: &str, original: &str) -> String {
    digest(format!("{}\0{song}\0{original}", manifest().version).as_bytes())
}
pub fn english(text: &str) -> bool {
    whatlang::detect(text).is_some_and(|i| i.lang() == whatlang::Lang::Eng && i.is_reliable())
}
pub struct Engine {
    encoder: Session,
    decoder: Session,
    source: sentencepiece_rs::SentencePieceProcessor,
    target: sentencepiece_rs::SentencePieceProcessor,
    vocab: HashMap<String, i64>,
    inverse: HashMap<i64, String>,
}
impl Engine {
    pub fn load() -> Result<Self, Error> {
        let p = fs::canonicalize(root())?;
        for a in manifest().files.iter().filter(|a| a.name != "runtime.zip") {
            verify(&p.join(&a.name), a)?;
        }
        let env = ort::init_from(p.join("onnxruntime.dll"))?
            .with_telemetry(false)
            .build()?;
        let session = |name| -> Result<Session, Error> {
            Ok(Session::builder(&env)?
                .with_intra_threads(2)
                .map_err(|e| e.to_string())?
                .with_inter_threads(1)
                .map_err(|e| e.to_string())?
                .commit_from_file(p.join(name))?)
        };
        let vocab: HashMap<String, i64> = serde_json::from_slice(&fs::read(p.join("vocab.json"))?)?;
        Ok(Self {
            encoder: session("encoder_model_quantized.onnx")?,
            decoder: session("decoder_model_merged_quantized.onnx")?,
            source: sentencepiece_rs::SentencePieceProcessor::open(p.join("source.spm"))?,
            target: sentencepiece_rs::SentencePieceProcessor::open(p.join("target.spm"))?,
            inverse: vocab.iter().map(|(k, v)| (*v, k.clone())).collect(),
            vocab,
        })
    }
    fn verse(&mut self, text: &str, cancel: &AtomicBool) -> Result<String, Error> {
        let pieces = self.source.encode(text)?;
        let mut translated = Vec::new();
        for chunk in pieces.chunks(240) {
            let mut ids = vec![self.vocab[">>pt_br<<"]];
            ids.extend(chunk.iter().map(|s| *self.vocab.get(s).unwrap_or(&1)));
            ids.push(0);
            let mask = vec![1i64; ids.len()];
            let encoded=self.encoder.run(ort::inputs!["input_ids"=>Tensor::from_array(([1,ids.len()],ids))?,"attention_mask"=>Tensor::from_array(([1,mask.len()],mask.clone()))?])?;
            let (shape, data) = encoded["last_hidden_state"].try_extract_tensor::<f32>()?;
            let hidden_shape = shape.to_vec();
            let hidden = data.to_vec();
            drop(encoded);
            let mut past: HashMap<String, (Vec<i64>, Vec<f32>)> = HashMap::new();
            let mut token = 65000i64;
            let mut result = Vec::new();
            let mut ended = false;
            for step in 0..512 {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Translation canceled".into());
                }
                let mut inputs: Vec<(String, DynValue)> = vec![
                    (
                        "input_ids".into(),
                        Tensor::from_array(([1, 1], vec![token]))?.into_dyn(),
                    ),
                    (
                        "encoder_attention_mask".into(),
                        Tensor::from_array(([1, mask.len()], mask.clone()))?.into_dyn(),
                    ),
                    (
                        "encoder_hidden_states".into(),
                        Tensor::from_array((hidden_shape.clone(), hidden.clone()))?.into_dyn(),
                    ),
                    (
                        "use_cache_branch".into(),
                        Tensor::from_array(([1], vec![step > 0]))?.into_dyn(),
                    ),
                ];
                for layer in 0..6 {
                    for side in ["decoder", "encoder"] {
                        for kind in ["key", "value"] {
                            let name = format!("past_key_values.{layer}.{side}.{kind}");
                            let (s, d) = past
                                .get(&name)
                                .cloned()
                                .unwrap_or((vec![1, 8, 0, 64], vec![]));
                            inputs.push((name, Tensor::from_array((s, d))?.into_dyn()));
                        }
                    }
                }
                let output = self.decoder.run(inputs)?;
                let (_, logits) = output["logits"].try_extract_tensor::<f32>()?;
                token = logits
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != 65000)
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .ok_or("Empty logits")?
                    .0 as i64;
                if token == 0 {
                    ended = true;
                    break;
                }
                result.push(token);
                for layer in 0..6 {
                    for side in ["decoder", "encoder"] {
                        if step > 0 && side == "encoder" {
                            continue;
                        }
                        for kind in ["key", "value"] {
                            let key = format!("{layer}.{side}.{kind}");
                            let (s, d) =
                                output[format!("present.{key}")].try_extract_tensor::<f32>()?;
                            past.insert(format!("past_key_values.{key}"), (s.to_vec(), d.to_vec()));
                        }
                    }
                }
            }
            if !ended {
                return Err("Translation exceeded decoder limit".into());
            }
            let words: Vec<&str> = result
                .iter()
                .filter_map(|id| self.inverse.get(id).map(String::as_str))
                .collect();
            translated.push(self.target.decode(&words)?);
        }
        Ok(translated.join(" "))
    }
    pub fn translate(&mut self, text: &str, cancel: &AtomicBool) -> Result<Translation, Error> {
        let mut memo = HashMap::new();
        let mut lines = Vec::new();
        for line in text.split('\n') {
            if line.trim().is_empty() {
                lines.push(String::new());
                continue;
            }
            let key = line.trim();
            if !memo.contains_key(key) {
                memo.insert(key.to_owned(), self.verse(key, cancel)?);
            }
            lines.push(memo[key].clone());
        }
        Ok(Translation {
            original: text.into(),
            translated: lines.join("\n"),
        })
    }
}
pub struct Job {
    pub key: String,
    pub cancel: Arc<AtomicBool>,
    pub rx: std::sync::mpsc::Receiver<Result<Translation, String>>,
}
impl Job {
    pub fn spawn(song: &str, original: String) -> Self {
        let key = cache_key(song, &original);
        let cache = root()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("lyrics")
            .join(format!("offline-v1-{key}.json"));
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        type Request = (
            PathBuf,
            String,
            Arc<AtomicBool>,
            std::sync::mpsc::Sender<Result<Translation, String>>,
        );
        static WORKER: std::sync::OnceLock<std::sync::mpsc::Sender<Request>> =
            std::sync::OnceLock::new();
        let sender = WORKER.get_or_init(|| {
            let (sender, receiver) = std::sync::mpsc::channel::<Request>();
            std::thread::spawn(move || {
                let mut engine: Option<Engine> = None;
                loop {
                    let mut request = match receiver.recv_timeout(Duration::from_secs(60)) {
                        Ok(r) => r,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            engine = None;
                            continue;
                        }
                        Err(_) => break,
                    };
                    while let Ok(latest) = receiver.try_recv() {
                        request = latest;
                    }
                    let (cache, original, c, tx) = request;
                    if c.load(Ordering::Relaxed) {
                        continue;
                    }
                    let result = (|| -> Result<Translation, Error> {
                        if let Ok(data) = fs::read(&cache) {
                            if let Ok(value) = serde_json::from_slice::<Translation>(&data) {
                                if value.original == original
                                    && value.translated.split('\n').count()
                                        == original.split('\n').count()
                                {
                                    return Ok(value);
                                }
                            }
                        }
                        if engine.is_none() {
                            engine = Some(Engine::load()?);
                        }
                        let result = engine.as_mut().unwrap().translate(&original, &c)?;
                        if !c.load(Ordering::Relaxed) {
                            fs::create_dir_all(cache.parent().unwrap())?;
                            let tmp = cache.with_extension("tmp");
                            fs::write(&tmp, serde_json::to_vec(&result)?)?;
                            fs::rename(tmp, cache)?;
                        }
                        Ok(result)
                    })();
                    let _ = tx.send(result.map_err(|e| e.to_string()));
                }
            });
            sender
        });
        let _ = sender.send((cache, original, c, tx));
        Self { key, cancel, rx }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_includes_original_and_song() {
        assert_ne!(cache_key("a", "hello"), cache_key("b", "hello"));
        assert_ne!(cache_key("a", "hello"), cache_key("a", "Hello"));
    }
    #[test]
    fn language_gate() {
        assert!(english(
            "This is a beautiful morning and I am walking down the street with my friends."
        ));
        assert!(!english(
            "Esta é uma linda manhã e estou caminhando na rua com meus amigos."
        ));
    }
    #[test]
    fn rejects_corrupted_asset() {
        let path = std::env::temp_dir().join(format!("nusa-model-test-{}", std::process::id()));
        fs::write(&path, b"test").unwrap();
        let asset = Asset {
            name: "test".into(),
            url: String::new(),
            size: 4,
            sha256: digest(b"test"),
        };
        assert!(verify(&path, &asset).is_ok());
        fs::write(&path, b"fail").unwrap();
        assert!(verify(&path, &asset).is_err());
        fs::write(&path, b"short").unwrap();
        assert!(verify(&path, &asset).is_err());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn indicator_tracks_origin_and_mode() {
        let c = crate::lyrics_view::LyricsContent {
            translation_origin: TranslationOrigin::Offline,
            ..Default::default()
        };
        assert_eq!(c.offline_label(0, false), "");
        assert_eq!(c.offline_label(1, false), "Tradução offline");
        assert_eq!(c.offline_label(2, true), "");
        assert_eq!(c.offline_label(3, true), "Offline translation");
    }
    #[test]
    fn old_settings_are_disabled_and_can_round_trip() {
        let mut s: crate::model::Settings = serde_json::from_str("{}").unwrap();
        assert!(!s.lyrics.offline_translation);
        s.lyrics.offline_translation = true;
        let text = serde_json::to_string(&s).unwrap();
        assert!(
            serde_json::from_str::<crate::model::Settings>(&text)
                .unwrap()
                .lyrics
                .offline_translation
        );
    }
    #[test]
    fn dropping_job_cancels_pending_result() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job {
            key: String::new(),
            cancel: cancel.clone(),
            rx,
        };
        drop(job);
        assert!(cancel.load(Ordering::Relaxed));
    }
    #[test]
    fn canceled_install_does_not_start_download() {
        assert!(install(&AtomicBool::new(true), |_, _| panic!(
            "no progress after cancellation"
        ))
        .is_err());
    }
}
