//! Every component on one page, for eyeballing CSS changes. Debug builds only (see `main`).

use axum::{Router, routing::get};
use maud::{Markup, html};

use super::{layout::*, ui::*};
use crate::auth::AuthSession;

/// `GET /_styleguide`. Only mounted in debug builds, behind login (see `main`).
pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new().route("/_styleguide", get(show))
}

/// The kitchen sink inside the real authenticated page frame.
async fn show(auth: AuthSession) -> Markup {
    let user = auth.user.expect("behind login_required");
    app_page(&user, Nav::Devices, "Styleguide", body())
}

/// Every component, with sample data. Public so `examples/styleguide.rs` can serve it without
/// the rest of the app.
pub fn body() -> Markup {
    let rows = vec![
        html! { tr { td { (status_dot(true, "online", "offline")) } td { "laptop" } td { (chips(["reef", "dev"])) }
        td.actions { (delete_button("/devices/1", "Remove laptop?", None)) } } },
        html! { tr { td { (status_dot(false, "online", "offline")) } td { "nas" } td { (chips([])) }
        td.actions { (delete_button("/devices/2", "Remove nas?", None)) } } },
    ];
    html! {
        h1 { "Styleguide" }
        (notice(Tone::Ok, "Saved."))
        (notice(Tone::Warn, "Heads up: this is a warning."))
        (notice(Tone::Danger, "`foo.reef` is already owned by another user."))
        (notice(Tone::Neutral, "Neutral note."))

        (panel("Table", Some(html! { a.btn.btn-sm href="#" { "Action" } }), true,
            data_table(&["", "Name", "Groups", ""], rows, "No devices.")))
        (panel("Empty table", None, true, data_table(&["Name"], vec![], "No devices yet.")))

        (panel("Form (row)", None, false, form("/dns", "#claims", "beforeend", FormLayout::Row, html! {
            (select("Kind", "kind", &[("reef", ".reef domain"), ("tld", "TLD")], "reef"))
            (text_field(&Field { label: "Name", name: "name", placeholder: Some("mything"), suffix: Some(".reef"), ..Default::default() }))
            (submit("Claim"))
        })))
        (panel("Form (stack, with error)", None, false, form("/x", "#x", "outerHTML", FormLayout::Stack, html! {
            (text_field(&Field { label: "Group name", name: "group", value: "reef", hint: Some("lowercase letters, digits, dashes"), error: Some("`reef` is reserved"), ..Default::default() }))
            (checkbox_group("Device groups", "groups", &[
                Choice { value: "reef", label: "reef", checked: true },
                Choice { value: "dev", label: "dev", checked: false },
                Choice { value: "ops", label: "ops", checked: true },
            ]))
            (submit("Save"))
        })))
        (panel("Misc", None, false, html! {
            p { (badge("neutral", Tone::Neutral)) " " (badge("ok", Tone::Ok)) " " (badge("warn", Tone::Warn)) " " (badge("danger", Tone::Danger)) }
            p { "Mono: " (mono("web.foo.reef")) " · " span.muted { "muted text" } }
            p { button.btn { "Default" } " " button.btn.btn-primary { "Primary" } " " button.btn.btn-danger { "Danger" } " " button.btn.btn-sm { "Small" } " " button.btn disabled { "Disabled" } }
        }))
    }
}
