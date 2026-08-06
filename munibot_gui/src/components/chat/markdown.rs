//! Renders one message's markdown content into `rsx!`.
//!
//! Code blocks are the centrepiece, per the programming use case: a
//! language label, a copy button, and syntax highlighting applied
//! client-side by a CDN-hosted highlighter (`App`'s own `document::Script`),
//! rather than shipping a full grammar set into the wasm bundle.

use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::prelude::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};

/// One still-open block, as the flat event stream is walked: the tag that
/// opened it (`None` only for the synthetic root, which has no matching
/// `End` event) and whatever children have been rendered into it so far.
type Frame<'a> = (Option<Tag<'a>>, Vec<Element>);

/// Renders `source` as markdown.
///
/// Extensions beyond plain CommonMark are deliberately minimal: just
/// strikethrough, since `~~text~~` is common and free to support. Tables,
/// footnotes, and task lists are not parsed, and raw inline HTML is dropped
/// rather than rendered, since a message body is never a trusted document.
pub fn render_markdown(source: &str) -> Element {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let mut stack: Vec<Frame<'_>> = vec![(None, Vec::new())];
    // code block content is accumulated as one raw string rather than
    // child elements: the copy button needs the exact source text, and the
    // highlighter needs a single unbroken string to point at, not a tree of
    // already-rendered nodes
    let mut code_accum: Option<String> = None;

    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(tag) => {
                if matches!(tag, Tag::CodeBlock(_)) {
                    code_accum = Some(String::new());
                }
                stack.push((Some(tag), Vec::new()));
            }
            Event::End(_) => {
                let (tag, children) = stack.pop().expect("balanced by the parser");
                let tag = tag.expect("only the root frame has no tag, and it's never popped");
                let rendered = match &tag {
                    Tag::CodeBlock(kind) => {
                        render_code_block(language_of(kind), code_accum.take().unwrap_or_default())
                    }
                    other => render_tag(other, children),
                };
                top(&mut stack).push(rendered);
            }
            Event::Text(text) => push_text(&mut stack, &mut code_accum, &text),
            Event::Code(text) => top(&mut stack).push(rsx! {
                code { class: "bg-slate-800 rounded px-1 py-0.5 text-sm", {text.into_string()} }
            }),
            Event::SoftBreak => top(&mut stack).push(rsx! { " " }),
            Event::HardBreak => top(&mut stack).push(rsx! {
                br {}
            }),
            Event::Rule => top(&mut stack).push(rsx! {
                hr { class: "my-2 border-slate-700" }
            }),
            // not supported yet: raw html and footnote/tasklist markers are
            // dropped rather than rendered, since a message body is never a
            // trusted document and there's nowhere else useful to put them
            Event::Html(_) | Event::FootnoteReference(_) | Event::TaskListMarker(_) => {}
        }
    }

    let (_, children) = stack.pop().expect("the root frame is always present");
    rsx! {
        div { class: "flex flex-col gap-2 [overflow-wrap:anywhere]",
            for child in children {
                {child}
            }
        }
    }
}

/// The current block's child list -- the top of the stack, which always has
/// at least the root frame in it.
fn top<'a, 'b>(stack: &'b mut [Frame<'a>]) -> &'b mut Vec<Element> {
    &mut stack
        .last_mut()
        .expect("the root frame is always present")
        .1
}

/// Appends text to whichever destination is active: the raw accumulator
/// while inside a code block (see [`render_markdown`]'s own comment), or a
/// rendered text child otherwise.
fn push_text(stack: &mut [Frame<'_>], code_accum: &mut Option<String>, text: &str) {
    if let Some(code) = code_accum {
        code.push_str(text);
    } else {
        top(stack).push(rsx! {
            {text.to_string()}
        });
    }
}

/// The fence's language tag, or `None` for an indented block or an empty
/// (unlabelled) fence.
fn language_of(kind: &CodeBlockKind<'_>) -> Option<String> {
    match kind {
        CodeBlockKind::Fenced(lang) if !lang.is_empty() => Some(lang.to_string()),
        _ => None,
    }
}

/// Renders every block/inline tag except [`Tag::CodeBlock`], which
/// [`render_markdown`] handles separately since it needs the raw
/// accumulated text rather than a list of already-rendered children.
///
/// Tables and footnote definitions are not supported yet: rendered as their
/// children with no special wrapper, rather than dropped outright, so at
/// least the text inside them isn't lost.
fn render_tag(tag: &Tag<'_>, children: Vec<Element>) -> Element {
    match tag {
        Tag::Paragraph => rsx! {
            p { class: "leading-relaxed",
                for child in children {
                    {child}
                }
            }
        },
        Tag::Heading(level, ..) => render_heading(*level, children),
        Tag::BlockQuote => rsx! {
            blockquote { class: "text-slate-300 border-s-4 border-slate-700 ps-4 italic",
                for child in children {
                    {child}
                }
            }
        },
        Tag::List(Some(start)) => rsx! {
            ol { class: "list-decimal ps-6", start: *start as i64,
                for child in children {
                    {child}
                }
            }
        },
        Tag::List(None) => rsx! {
            ul { class: "list-disc ps-6",
                for child in children {
                    {child}
                }
            }
        },
        Tag::Item => rsx! {
            li {
                for child in children {
                    {child}
                }
            }
        },
        Tag::Emphasis => rsx! {
            em {
                for child in children {
                    {child}
                }
            }
        },
        Tag::Strong => rsx! {
            strong {
                for child in children {
                    {child}
                }
            }
        },
        Tag::Strikethrough => rsx! {
            del {
                for child in children {
                    {child}
                }
            }
        },
        Tag::Link(_, url, _) => rsx! {
            a {
                class: "link link-primary",
                href: url.to_string(),
                target: "_blank",
                rel: "noopener noreferrer",
                for child in children {
                    {child}
                }
            }
        },
        Tag::Image(_, url, title) => rsx! {
            img {
                src: url.to_string(),
                alt: title.to_string(),
                class: "max-w-full rounded-box",
            }
        },
        Tag::CodeBlock(_) => unreachable!("render_markdown handles code blocks itself"),
        _ => rsx! {
            for child in children {
                {child}
            }
        },
    }
}

fn render_heading(level: HeadingLevel, children: Vec<Element>) -> Element {
    match level {
        HeadingLevel::H1 => rsx! {
            h1 { class: "font-black text-2xl",
                for child in children {
                    {child}
                }
            }
        },
        HeadingLevel::H2 => rsx! {
            h2 { class: "font-black text-xl",
                for child in children {
                    {child}
                }
            }
        },
        HeadingLevel::H3 => rsx! {
            h3 { class: "font-bold text-lg",
                for child in children {
                    {child}
                }
            }
        },
        HeadingLevel::H4 => rsx! {
            h4 { class: "font-bold",
                for child in children {
                    {child}
                }
            }
        },
        HeadingLevel::H5 => rsx! {
            h5 { class: "font-bold",
                for child in children {
                    {child}
                }
            }
        },
        HeadingLevel::H6 => rsx! {
            h6 { class: "font-bold",
                for child in children {
                    {child}
                }
            }
        },
    }
}

/// Backs each code block's DOM `id`, so the highlighter can be pointed at
/// exactly the element that was just mounted (see the `onmounted` handler
/// below) without needing a raw element handle.
static NEXT_CODE_BLOCK_ID: AtomicU64 = AtomicU64::new(0);

/// Renders one fenced code block: a header with the language label and a
/// copy button, then the code itself, highlighted client-side once mounted.
fn render_code_block(language: Option<String>, code: String) -> Element {
    let id = format!(
        "chat-code-{}",
        NEXT_CODE_BLOCK_ID.fetch_add(1, Ordering::Relaxed)
    );
    let label = language.clone().unwrap_or_else(|| "text".to_string());
    let lang_class = language
        .map(|lang| format!("language-{lang}"))
        .unwrap_or_default();
    let highlight_id = id.clone();

    rsx! {
        div { class: "overflow-hidden rounded-box bg-slate-950",
            div { class: "flex items-center justify-between bg-slate-900 px-4 py-1 text-xs text-slate-400",
                span { {label} }
                CopyButton { text: code.clone() }
            }
            pre { class: "p-4 overflow-x-auto text-sm",
                code {
                    id,
                    class: lang_class,
                    // hljs.highlightElement mutates the element in place, so this
                    // only needs to run once, right after the element actually
                    // exists -- looked up by id rather than through a raw element
                    // handle, since that's all `document::eval`'s js-side needs
                    onmounted: move |_| {
                        document::eval(
                            &format!(
                                "if (window.hljs) {{ const el = document.getElementById('{highlight_id}'); if (el) window.hljs.highlightElement(el); }}",
                            ),
                        );
                    },
                    {code}
                }
            }
        }
    }
}

/// A small button that copies `text` to the clipboard, showing a brief
/// "copied!" confirmation.
///
/// The copy itself, and the confirmation's timeout, both go through
/// `document::eval`'s two-way channel rather than any JS string
/// interpolation: `text` reaches the browser via `dioxus.send`/`recv`, never
/// spliced into a script as a string literal, so pasted code containing
/// quotes or backticks can never break out of it.
#[component]
fn CopyButton(text: String) -> Element {
    let mut copied = use_signal(|| false);

    let on_click = move |_| {
        let text = text.clone();
        spawn(async move {
            let mut eval = document::eval(
                "const text = await dioxus.recv(); await navigator.clipboard.writeText(text); \
                 await new Promise((resolve) => setTimeout(resolve, 1500)); dioxus.send(true);",
            );
            if eval.send(text).is_ok() {
                copied.set(true);
                let _ = eval.recv::<bool>().await;
                copied.set(false);
            }
        });
    };

    if *copied.read() {
        rsx! {
            button { class: "btn btn-ghost btn-xs gap-1", disabled: true,
                i { class: "ph-duotone ph-check" }
                "copied!"
            }
        }
    } else {
        rsx! {
            button { class: "btn btn-ghost btn-xs gap-1", onclick: on_click,
                i { class: "ph-duotone ph-copy" }
                "copy"
            }
        }
    }
}
