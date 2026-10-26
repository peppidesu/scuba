//! Small, composable UI pieces. Every function returns a self-contained `Markup` fragment,
//! suitable for use in a full page or as an htmx response. User-supplied text is always
//! passed through maud's escaping — never build HTML by string concatenation here.

use maud::{Markup, Render, html};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Ok,
    Warn,
    Danger,
}

impl Tone {
    fn suffix(self) -> &'static str {
        match self {
            Tone::Neutral => "",
            Tone::Ok => " badge-ok",
            Tone::Warn => " badge-warn",
            Tone::Danger => " badge-danger",
        }
    }
    fn notice_class(self) -> &'static str {
        match self {
            Tone::Neutral => "notice",
            Tone::Ok => "notice notice-ok",
            Tone::Warn => "notice notice-warn",
            Tone::Danger => "notice notice-danger",
        }
    }
}

// ---- display ----

pub fn badge(text: &str, tone: Tone) -> Markup {
    html! { span class={ "badge" (tone.suffix()) } { (text) } }
}

/// A row of small tags (e.g. a device's groups). Renders nothing visible when empty except a dash.
pub fn chips<'a>(items: impl IntoIterator<Item = &'a str>) -> Markup {
    let items: Vec<&str> = items.into_iter().collect();
    html! {
        @if items.is_empty() {
            span.muted { "—" }
        } @else {
            span.chips { @for i in items { (badge(i, Tone::Neutral)) } }
        }
    }
}

pub fn status_dot(on: bool, on_label: &str, off_label: &str) -> Markup {
    html! {
        span.status { span class={ "dot" @if on { " dot-on" } } aria-hidden="true" {} (if on { on_label } else { off_label }) }
    }
}

pub fn mono(text: &str) -> Markup {
    html! { code { (text) } }
}

/// Inline message. Errors/warnings use role="alert" so assistive tech announces them when swapped in.
pub fn notice(tone: Tone, msg: impl Render) -> Markup {
    let alert = matches!(tone, Tone::Danger | Tone::Warn);
    html! {
        div class=(tone.notice_class()) role=(if alert { "alert" } else { "status" }) { (msg) }
    }
}

pub fn empty(msg: &str) -> Markup {
    html! { div.empty { (msg) } }
}

// ---- structure ----

/// Bordered section with a title bar and optional right-aligned actions.
/// `flush` removes body padding (use for tables).
pub fn panel(title: &str, actions: Option<Markup>, flush: bool, body: Markup) -> Markup {
    html! {
        section.panel {
            div.panel-head { h2 { (title) } @if let Some(a) = actions { div { (a) } } }
            div class={ "panel-body" @if flush { " flush" } } { (body) }
        }
    }
}

/// `rows` are complete `<tr>` fragments. Shows `empty_msg` instead when there are none.
pub fn data_table(headers: &[&str], rows: Vec<Markup>, empty_msg: &str) -> Markup {
    if rows.is_empty() {
        return empty(empty_msg);
    }
    html! {
        table.data {
            thead { tr { @for h in headers { th { (h) } } } }
            tbody { @for r in rows { (r) } }
        }
    }
}

// ---- actions ----

pub fn submit(label: &str) -> Markup {
    html! { button.btn.btn-primary type="submit" { (label) } }
}

/// DELETE `url`, ask for confirmation, then remove the closest `<tr>` (default) or swap `target`.
/// `target`/`swap` follow htmx syntax; pass `None` for the row-removal default.
pub fn delete_button(url: &str, confirm: &str, target: Option<&str>) -> Markup {
    let (target, swap) = match target {
        Some(t) => (t, "outerHTML"),
        None => ("closest tr", "outerHTML swap:150ms"),
    };
    html! {
        button.btn.btn-sm.btn-danger type="button"
            hx-delete=(url) hx-confirm=(confirm) hx-target=(target) hx-swap=(swap) { "Remove" }
    }
}

// ---- forms ----

/// htmx form: POSTs to `url`, response replaces `target` (swap = outerHTML unless the target
/// is a list you append to, in which case pass your own swap).
pub fn form(url: &str, target: &str, swap: &str, layout: FormLayout, body: Markup) -> Markup {
    let class = match layout {
        FormLayout::Row => "form-row",
        FormLayout::Stack => "form-stack",
    };
    html! { form class=(class) hx-post=(url) hx-target=(target) hx-swap=(swap) { (body) } }
}

#[derive(Clone, Copy, Debug)]
pub enum FormLayout {
    Row,
    Stack,
}

#[derive(Default)]
pub struct Field<'a> {
    pub label: &'a str,
    pub name: &'a str,
    pub value: &'a str,
    pub placeholder: Option<&'a str>,
    pub hint: Option<&'a str>,
    pub error: Option<&'a str>,
    /// Fixed text shown after the input (e.g. ".reef").
    pub suffix: Option<&'a str>,
    pub required: bool,
}

pub fn text_field(f: &Field) -> Markup {
    let id = format!("f-{}", f.name);
    let err_id = format!("{id}-err");
    let invalid = f.error.is_some();
    let input = html! {
        input type="text" id=(id) name=(f.name) value=(f.value) placeholder=[f.placeholder]
            required[f.required] autocomplete="off" spellcheck="false"
            aria-invalid=[invalid.then_some("true")] aria-describedby=[invalid.then_some(err_id.as_str())];
    };
    html! {
        div.field {
            label for=(id) { (f.label) }
            @if let Some(s) = f.suffix {
                div.input-suffix { (input) span { (s) } }
            } @else {
                (input)
            }
            @if let Some(h) = f.hint { span.hint { (h) } }
            @if let Some(e) = f.error { span.error id=(err_id) { (e) } }
        }
    }
}

pub fn select(label: &str, name: &str, options: &[(&str, &str)], selected: &str) -> Markup {
    let id = format!("f-{name}");
    html! {
        div.field {
            label for=(id) { (label) }
            select id=(id) name=(name) {
                @for (value, text) in options { option value=(value) selected[*value == selected] { (text) } }
            }
        }
    }
}

/// One checkbox in a [`checkbox_group`].
pub struct Choice<'a> {
    pub value: &'a str,
    pub label: &'a str,
    pub checked: bool,
}

/// A set of checkboxes sharing one `name`; submitted as repeated form fields.
pub fn checkbox_group(legend: &str, name: &str, choices: &[Choice]) -> Markup {
    html! {
        fieldset.field {
            legend { (legend) }
            div.checks {
                @for c in choices {
                    label { input type="checkbox" name=(name) value=(c.value) checked[c.checked]; (c.label) }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(m: Markup) -> String {
        m.into_string()
    }

    const XSS: &str = "<script>alert(1)</script>";

    #[test]
    fn user_text_is_escaped_everywhere() {
        let outputs = [
            s(badge(XSS, Tone::Neutral)),
            s(chips([XSS])),
            s(notice(Tone::Danger, XSS)),
            s(mono(XSS)),
            s(empty(XSS)),
            s(panel(XSS, None, false, html! {})),
            s(data_table(&[XSS], vec![html! { tr { td { "x" } } }], "")),
            s(status_dot(true, XSS, XSS)),
            s(delete_button("/x", XSS, None)),
            s(text_field(&Field {
                label: XSS,
                name: "n",
                value: XSS,
                hint: Some(XSS),
                error: Some(XSS),
                suffix: Some(XSS),
                ..Default::default()
            })),
            s(select(XSS, "n", &[(XSS, XSS)], XSS)),
            s(checkbox_group(
                XSS,
                "n",
                &[Choice {
                    value: XSS,
                    label: XSS,
                    checked: true,
                }],
            )),
        ];
        for o in outputs {
            assert!(!o.contains("<script>"), "unescaped: {o}");
        }
    }

    #[test]
    fn attribute_values_cannot_break_out_of_quotes() {
        let evil = "\"><img src=x onerror=alert(1)>";
        let out = s(text_field(&Field {
            label: "l",
            name: "n",
            value: evil,
            placeholder: Some(evil),
            ..Default::default()
        }));
        assert!(!out.contains("<img"), "{out}");
        let out = s(delete_button(evil, evil, Some(evil)));
        assert!(!out.contains("<img"), "{out}");
    }

    #[test]
    fn badge_tone_maps_to_class() {
        assert!(s(badge("a", Tone::Ok)).contains("badge badge-ok"));
        assert!(s(badge("a", Tone::Danger)).contains("badge-danger"));
        let plain = s(badge("a", Tone::Neutral));
        assert!(plain.contains("class=\"badge\""), "{plain}");
    }

    #[test]
    fn chips_show_a_dash_when_empty() {
        assert!(s(chips([])).contains("—"));
        let two = s(chips(["a", "b"]));
        assert_eq!(two.matches("class=\"badge\"").count(), 2);
    }

    #[test]
    fn notice_role_depends_on_severity() {
        assert!(s(notice(Tone::Danger, "x")).contains("role=\"alert\""));
        assert!(s(notice(Tone::Warn, "x")).contains("role=\"alert\""));
        assert!(s(notice(Tone::Ok, "x")).contains("role=\"status\""));
        assert!(s(notice(Tone::Neutral, "x")).contains("role=\"status\""));
    }

    #[test]
    fn status_dot_reflects_state() {
        let on = s(status_dot(true, "online", "offline"));
        assert!(on.contains("dot-on") && on.contains("online") && !on.contains("offline"));
        let off = s(status_dot(false, "online", "offline"));
        assert!(!off.contains("dot-on") && off.contains("offline"));
    }

    #[test]
    fn table_renders_rows_or_the_empty_message() {
        let t = s(data_table(
            &["A", "B"],
            vec![html! { tr { td { "1" } td { "2" } } }],
            "nothing",
        ));
        assert!(
            t.contains("<th>A</th><th>B</th>")
                && t.contains("<td>1</td>")
                && !t.contains("nothing")
        );
        let e = s(data_table(&["A"], vec![], "nothing here"));
        assert!(e.contains("nothing here") && !e.contains("<table"));
    }

    #[test]
    fn panel_actions_and_flush() {
        let p = s(panel("T", Some(html! { "ACT" }), true, html! { "BODY" }));
        assert!(p.contains("<h2>T</h2>") && p.contains("ACT") && p.contains("panel-body flush"));
        let p = s(panel("T", None, false, html! { "BODY" }));
        assert!(!p.contains("flush") && p.contains("class=\"panel-body\""));
    }

    #[test]
    fn delete_button_defaults_to_removing_its_row_with_confirmation() {
        let b = s(delete_button("/dns/3", "Release corp?", None));
        assert!(b.contains("hx-delete=\"/dns/3\"") && b.contains("hx-confirm=\"Release corp?\""));
        assert!(
            b.contains("hx-target=\"closest tr\"")
                && b.contains("hx-swap=\"outerHTML swap:150ms\"")
        );
        let b = s(delete_button("/dns/3", "?", Some("#claims")));
        assert!(b.contains("hx-target=\"#claims\"") && b.contains("hx-swap=\"outerHTML\""));
        assert!(
            b.contains("type=\"button\""),
            "must never submit an enclosing form"
        );
    }

    #[test]
    fn form_wires_htmx_and_layout() {
        let f = s(form(
            "/groups",
            "#groups",
            "beforeend",
            FormLayout::Row,
            html! { "x" },
        ));
        assert!(
            f.contains("hx-post=\"/groups\"")
                && f.contains("hx-target=\"#groups\"")
                && f.contains("hx-swap=\"beforeend\"")
        );
        assert!(f.contains("form-row"));
        assert!(
            s(form("/x", "#t", "outerHTML", FormLayout::Stack, html! {})).contains("form-stack")
        );
    }

    #[test]
    fn text_field_links_label_input_and_error() {
        let out = s(text_field(&Field {
            label: "Name",
            name: "name",
            error: Some("taken"),
            ..Default::default()
        }));
        assert!(out.contains("<label for=\"f-name\">Name</label>"));
        assert!(out.contains("id=\"f-name\"") && out.contains("name=\"name\""));
        assert!(
            out.contains("aria-invalid=\"true\"")
                && out.contains("aria-describedby=\"f-name-err\"")
        );
        assert!(out.contains("id=\"f-name-err\"") && out.contains("taken"));
    }

    #[test]
    fn text_field_without_error_has_no_invalid_markers() {
        let out = s(text_field(&Field {
            label: "Name",
            name: "name",
            ..Default::default()
        }));
        assert!(
            !out.contains("aria-invalid")
                && !out.contains("aria-describedby")
                && !out.contains("required")
        );
    }

    #[test]
    fn text_field_optional_parts() {
        let out = s(text_field(&Field {
            label: "L",
            name: "n",
            value: "v",
            placeholder: Some("ph"),
            hint: Some("h"),
            suffix: Some(".reef"),
            required: true,
            ..Default::default()
        }));
        for needle in [
            "value=\"v\"",
            "placeholder=\"ph\"",
            "class=\"hint\"",
            "input-suffix",
            ".reef",
            "required",
        ] {
            assert!(out.contains(needle), "missing {needle}: {out}");
        }
    }

    #[test]
    fn select_marks_only_the_selected_option() {
        let out = s(select(
            "Type",
            "rtype",
            &[("A", "A"), ("AAAA", "AAAA")],
            "AAAA",
        ));
        assert_eq!(out.matches("selected").count(), 1);
        assert!(out.contains("<option value=\"AAAA\" selected>"), "{out}");
    }

    #[test]
    fn checkbox_group_shares_a_name_and_marks_checked() {
        let out = s(checkbox_group(
            "Groups",
            "groups",
            &[
                Choice {
                    value: "reef",
                    label: "reef",
                    checked: true,
                },
                Choice {
                    value: "dev",
                    label: "dev",
                    checked: false,
                },
            ],
        ));
        assert_eq!(out.matches("name=\"groups\"").count(), 2);
        assert_eq!(out.matches("checked").count(), 1);
        assert!(out.contains("<legend>Groups</legend>"));
    }
}
