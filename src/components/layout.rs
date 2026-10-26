//! Page shells: the anonymous one (login errors, logged-out) and the authenticated app frame.

use maud::{DOCTYPE, Markup, html};

use crate::db::users::User;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The sections in the top navigation.
pub enum Nav {
    Devices,
    Dns,
    Groups,
}

impl Nav {
    /// In display order.
    pub const ALL: [Nav; 3] = [Nav::Devices, Nav::Dns, Nav::Groups];

    /// Where the link goes.
    pub fn href(self) -> &'static str {
        match self {
            Nav::Devices => "/devices",
            Nav::Dns => "/dns",
            Nav::Groups => "/groups",
        }
    }

    /// The link text.
    pub fn label(self) -> &'static str {
        match self {
            Nav::Devices => "Devices",
            Nav::Dns => "DNS",
            Nav::Groups => "Groups",
        }
    }
}

// htmx 2 does not swap 4xx/5xx responses by default. We want inline error fragments (sent with
// HX-Retarget / HX-Reswap) to render, so swap everything except 204.
const HTMX_CONFIG: &str = r#"{
    "responseHandling": [
        { "code": "204", "swap": false },
        { "code": "[23]..", "swap": true },
        { "code": "[45]..", "swap": true, "error": true }
    ]
}"#;

/// The HTML every page shares: head (CSS, htmx, config), then `header`, then `body` as the main content.
fn shell(title: &str, header: Markup, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="htmx-config" content=(HTMX_CONFIG);
                title { (title) " · scuba" }
                link rel="stylesheet" href="/static/app.css";
                script src="/static/htmx.min.js" defer {}
            }
            body hx-boost="true" {
                (header)
                main { (body) }
            }
        }
    }
}

/// Shell without navigation, for visitors who aren't logged in.
pub fn page(title: &str, body: Markup) -> Markup {
    let header = html! {
        header.topbar { div.topbar-inner { a.brand href="/" { "scuba" } } }
    };
    shell(title, header, body)
}

/// Authenticated frame: nav with the current section marked, who's logged in, logout.
pub fn app_page(user: &User, active: Nav, title: &str, body: Markup) -> Markup {
    let header = html! {
        header.topbar {
            div.topbar-inner {
                a.brand href="/" { "scuba" }
                nav.nav {
                    @for n in Nav::ALL {
                        a href=(n.href()) aria-current=[(n == active).then_some("page")] {
                            (n.label())
                        }
                    }
                }
                div.userbox {
                    span title=(user.email) { (user.display_name) }
                    form method="post" action="/logout" hx-boost="false" {
                        button.btn.btn-sm type="submit" { "Log out" }
                    }
                }
            }
        }
    };
    shell(title, header, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> User {
        User {
            id: 1,
            oidc_sub: "s".into(),
            email: "a@example.com".into(),
            display_name: "alice".into(),
            netbird_user_id: None,
        }
    }

    #[test]
    fn anonymous_page_has_no_nav_or_user() {
        let out = page("Login", html! { p { "hi" } }).into_string();
        assert!(out.contains("<title>Login · scuba</title>") && out.contains("<p>hi</p>"));
        assert!(!out.contains("class=\"nav\"") && !out.contains("/logout"));
    }

    #[test]
    fn page_includes_assets_htmx_boost_and_response_config() {
        let out = page("T", html! {}).into_string();
        assert!(out.contains("<!DOCTYPE html>") && out.contains("lang=\"en\""));
        assert!(
            out.contains("href=\"/static/app.css\"") && out.contains("src=\"/static/htmx.min.js\"")
        );
        assert!(out.contains("hx-boost=\"true\""));
        assert!(out.contains("name=\"htmx-config\"") && out.contains("responseHandling"));
    }

    #[test]
    fn htmx_config_is_valid_json_and_swaps_error_responses() {
        let v: serde_json::Value = serde_json::from_str(HTMX_CONFIG).unwrap();
        let rules = v["responseHandling"].as_array().unwrap();
        let swaps = |code: &str| {
            rules
                .iter()
                .find(|r| r["code"] == code)
                .map(|r| r["swap"].as_bool().unwrap())
        };
        assert_eq!(swaps("204"), Some(false));
        assert_eq!(swaps("[45].."), Some(true));
    }

    #[test]
    fn app_page_marks_exactly_the_active_section() {
        for active in Nav::ALL {
            let out = app_page(&user(), active, "T", html! {}).into_string();
            assert_eq!(
                out.matches("aria-current=\"page\"").count(),
                1,
                "{active:?}"
            );
            let expected = format!(
                "<a href=\"{}\" aria-current=\"page\">{}</a>",
                active.href(),
                active.label()
            );
            assert!(out.contains(&expected), "{expected} not in {out}");
        }
    }

    #[test]
    fn app_page_lists_all_sections_and_the_user() {
        let out = app_page(&user(), Nav::Dns, "T", html! { "BODY" }).into_string();
        for n in Nav::ALL {
            assert!(out.contains(n.href()));
        }
        assert!(
            out.contains("alice")
                && out.contains("title=\"a@example.com\"")
                && out.contains("BODY")
        );
    }

    #[test]
    fn logout_is_a_post_form_that_bypasses_boost() {
        let out = app_page(&user(), Nav::Devices, "T", html! {}).into_string();
        assert!(
            out.contains("method=\"post\"")
                && out.contains("action=\"/logout\"")
                && out.contains("hx-boost=\"false\"")
        );
    }

    #[test]
    fn user_supplied_names_are_escaped() {
        let mut u = user();
        u.display_name = "<script>x</script>".into();
        u.email = "\"><img src=x>".into();
        let out = app_page(&u, Nav::Devices, "<b>t</b>", html! {}).into_string();
        assert!(
            !out.contains("<script>x") && !out.contains("<img") && !out.contains("<b>t</b>"),
            "{out}"
        );
    }
}
