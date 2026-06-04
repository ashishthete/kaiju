//! Upload a file into an agent's working directory so the agent can read it by
//! path. Works for any file type, including images, which can't be streamed
//! through the terminal. Used by the dashboard's drag-and-drop.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::path::PathBuf;

use crate::server::AppState;

/// Subdirectory (inside the agent's run dir) where uploads are written.
const UPLOAD_DIR: &str = ".kaiju-uploads";

#[derive(Deserialize)]
pub struct UploadQuery {
    /// Original file name; only its basename is used.
    name: String,
}

/// `POST /agents/:id/files?name=<file>` with the raw file bytes as the body.
///
/// Writes the file into the agent's working directory (its worktree when
/// isolated, otherwise its workspace) and returns the relative path the agent
/// can open.
pub async fn upload_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> impl IntoResponse {
    let agent = match state.store.get(&id) {
        Some(a) => a,
        None => return Err((StatusCode::NOT_FOUND, "agent not found".to_string())),
    };

    let name = sanitize_filename(&q.name);
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "invalid file name".to_string()));
    }

    let run_dir = agent.worktree_path.clone().unwrap_or(agent.workspace);
    let rel = PathBuf::from(UPLOAD_DIR).join(&name);
    let dest = run_dir.join(&rel);

    let write = (|| -> std::io::Result<()> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, &body)
    })();

    match write {
        Ok(()) => Ok(Json(serde_json::json!({ "path": rel.to_string_lossy() }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// Reduce an arbitrary name to a safe basename: strips directories (no path
/// traversal) and any character that isn't alphanumeric, dot, dash, underscore,
/// or space.
fn sanitize_filename(name: &str) -> String {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    base.chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_paths_and_unsafe_chars() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a/b/c.txt"), "c.txt");
        assert_eq!(sanitize_filename("my photo.png"), "my photo.png");
        assert_eq!(sanitize_filename("weird;|name.rs"), "weirdname.rs");
        assert_eq!(sanitize_filename("/"), "");
        assert_eq!(sanitize_filename(""), "");
    }
}
