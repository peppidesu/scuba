//! Static files compiled into the binary. Public (no login) — the login pages need them too.

use axum::{
    Router,
    http::{HeaderValue, header},
    response::IntoResponse,
    routing::get,
};

const CSS: &str = include_str!("../../static/app.css");
const HTMX: &str = include_str!("../../static/htmx.min.js");

/// A static file response: the right content type, and cached for 5 minutes in release builds
/// (never in debug builds, so CSS edits show up on refresh).
fn served(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            // no stale CSS while developing; cache in release
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(if cfg!(debug_assertions) {
                    "no-cache"
                } else {
                    "public, max-age=300"
                }),
            ),
        ],
        body,
    )
}

/// `/static/app.css` and `/static/htmx.min.js`.
pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route(
            "/static/app.css",
            get(|| async { served("text/css; charset=utf-8", CSS) }),
        )
        .route(
            "/static/htmx.min.js",
            get(|| async { served("text/javascript; charset=utf-8", HTMX) }),
        )
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    use super::*;

    async fn fetch(path: &str) -> (axum::http::StatusCode, axum::http::HeaderMap, String) {
        let r = router::<()>()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (p, b) = r.into_parts();
        (
            p.status,
            p.headers,
            String::from_utf8(axum::body::to_bytes(b, usize::MAX).await.unwrap().to_vec()).unwrap(),
        )
    }

    #[tokio::test]
    async fn css_is_served_with_type_and_caching() {
        let (status, h, body) = fetch("/static/app.css").await;
        assert_eq!(status, 200);
        assert_eq!(h["content-type"], "text/css; charset=utf-8");
        let cc = h["cache-control"].to_str().unwrap();
        assert!(cc == "no-cache" || cc.contains("max-age"), "{cc}");
        assert!(body.contains("--radius"));
    }

    #[tokio::test]
    async fn htmx_is_served_as_javascript() {
        let (status, h, body) = fetch("/static/htmx.min.js").await;
        assert_eq!(status, 200);
        assert_eq!(h["content-type"], "text/javascript; charset=utf-8");
        assert!(body.contains("htmx"));
    }

    #[tokio::test]
    async fn unknown_static_paths_404() {
        assert_eq!(fetch("/static/nope.js").await.0, 404);
    }

    /// Every customization knob the docs promise must exist in the stylesheet.
    #[test]
    fn stylesheet_exposes_the_documented_tokens() {
        for token in [
            "--font",
            "--text-size",
            "--radius",
            "--gap",
            "--pad-x",
            "--pad-y",
            "--control-height",
            "--content-width",
            "--bg",
            "--surface",
            "--border",
            "--text",
            "--muted",
            "--accent",
            "--ok",
            "--warn",
            "--danger",
        ] {
            assert!(
                CSS.contains(&format!("{token}:")),
                "token {token} undefined"
            );
        }
    }

    #[test]
    fn corners_are_only_slightly_rounded_by_default() {
        let radius = CSS
            .lines()
            .find(|l| l.trim_start().starts_with("--radius:"))
            .unwrap();
        let px: u32 = radius
            .split(':')
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim()
            .trim_end_matches("px")
            .parse()
            .expect("radius in px");
        assert!(px <= 3, "{radius}");
    }

    #[test]
    fn components_use_tokens_not_hardcoded_radii() {
        let hard: Vec<_> = CSS
            .lines()
            .filter(|l| {
                l.contains("border-radius:")
                    && !l.contains("var(--radius)")
                    && !l.contains("50%")
                    && !l.contains("0 var(--radius) var(--radius) 0")
            })
            .collect();
        assert!(hard.is_empty(), "{hard:?}");
    }
}
