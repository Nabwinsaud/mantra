use std::{
    net::TcpListener as StdTcpListener,
    process::{Command, Stdio},
    sync::Arc,
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    http::{HeaderValue, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use uuid::Uuid;

use crate::database::SchemaDiagram;

const INDEX_HTML: &str = include_str!("../schema-viewer/dist/index.html");

pub struct SchemaViewerLaunch {
    pub url: String,
    pub browser_opened: bool,
}

pub fn open(diagram: SchemaDiagram) -> Result<SchemaViewerLaunch> {
    let listener =
        StdTcpListener::bind(("127.0.0.1", 0)).context("could not bind the local schema viewer")?;
    listener
        .set_nonblocking(true)
        .context("could not configure the local schema viewer")?;
    let address = listener
        .local_addr()
        .context("could not read the schema viewer address")?;
    let listener = tokio::net::TcpListener::from_std(listener)
        .context("could not start the local schema viewer")?;
    let token = Uuid::new_v4().simple().to_string();
    let index_path = format!("/{token}");
    let schema_path = format!("/{token}/schema");
    let schema = Arc::new(diagram);

    let router = Router::new()
        .route(&index_path, get(|| async { secured(Html(INDEX_HTML)) }))
        .route(
            &schema_path,
            get({
                let schema = Arc::clone(&schema);
                move || {
                    let schema = Arc::clone(&schema);
                    async move { secured(Json(schema.as_ref().clone())) }
                }
            }),
        );
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::warn!(%error, "schema viewer stopped");
        }
    });

    let url = format!("http://{address}/{token}");
    let browser_opened = open_browser(&url);
    Ok(SchemaViewerLaunch {
        url,
        browser_opened,
    })
}

fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let command = ("open", vec![url]);
    #[cfg(target_os = "linux")]
    let command = ("xdg-open", vec![url]);
    #[cfg(target_os = "windows")]
    let command = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return false;

    Command::new(command.0)
        .args(command.1)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

fn secured(response: impl IntoResponse) -> Response {
    let mut response = response.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src data:; font-src 'self'",
        ),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_viewer_is_a_built_single_page_app() {
        assert!(INDEX_HTML.contains("<div id=\"root\"></div>"));
        assert!(INDEX_HTML.contains("Find a table, column, or type"));
        assert!(!INDEX_HTML.contains("https://fonts.googleapis.com"));
    }
}
