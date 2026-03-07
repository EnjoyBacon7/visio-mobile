use ort::session::Session;
use std::path::Path;
use std::sync::OnceLock;

static SESSION: OnceLock<Session> = OnceLock::new();

/// Load the selfie segmentation ONNX model from the given path.
/// Called once at app startup or first blur enable.
pub fn load_model(model_path: &Path) -> Result<(), String> {
    let mut builder = Session::builder()
        .map_err(|e| format!("ort session builder: {e}"))?
        .with_intra_threads(2)
        .map_err(|e| format!("ort threads: {e}"))?;
    let session = builder
        .commit_from_file(model_path)
        .map_err(|e| format!("ort load model: {e}"))?;
    SESSION.set(session).map_err(|_| "model already loaded".into())
}

pub fn get_session() -> Option<&'static Session> {
    SESSION.get()
}
