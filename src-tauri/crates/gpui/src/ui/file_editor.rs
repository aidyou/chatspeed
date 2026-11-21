//! Native text editing for files selected from the authorized directory tree.

use gpui_kit::prelude::FluentBuilder;
#[cfg(test)]
use std::fs;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use gpui_kit::component::{
    h_flex,
    input::{Editor, EditorState},
    v_flex, ActiveTheme,
};
use gpui_kit::*;
use rust_i18n::t;

use crate::Observer;

const MAX_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct FileEditor {
    document: Option<Document>,
    loading: bool,
    saving: bool,
    discard: bool,
    error: Option<String>,
}

struct Document {
    path: PathBuf,
    original: String,
    baseline: String,
    input: Entity<EditorState>,
}

fn authorized(path: &Path, roots: &[String]) -> io::Result<PathBuf> {
    let path = path.canonicalize()?;
    if !roots.iter().any(|root| {
        Path::new(root)
            .canonicalize()
            .is_ok_and(|root| root.is_dir() && path.starts_with(root))
    }) {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    if !path.metadata()?.is_file() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(path)
}

fn read_text(path: &Path) -> io::Result<String> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES || bytes.contains(&0) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    String::from_utf8(bytes).map_err(|_| io::ErrorKind::InvalidData.into())
}

fn save_text(path: &Path, roots: &[String], original: &str, text: &str) -> io::Result<()> {
    if text.len() as u64 > MAX_BYTES || text.contains('\0') {
        return Err(io::ErrorKind::InvalidData.into());
    }
    if authorized(path, roots)? != path {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file()
        .set_permissions(path.metadata()?.permissions())?;
    temp.as_file().sync_all()?;
    if authorized(path, roots)? != path || read_text(path)? != original {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn language(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" => "rust".into(),
        "js" | "mjs" | "cjs" => "javascript".into(),
        "ts" | "mts" | "cts" => "typescript".into(),
        "py" => "python".into(),
        "yml" => "yaml".into(),
        "md" => "markdown".into(),
        "sh" | "bash" => "bash".into(),
        "" => "plaintext".into(),
        _ => ext,
    }
}

pub(super) fn open(
    this: &mut Observer,
    path: String,
    window: &mut Window,
    cx: &mut Context<Observer>,
) {
    let editor = &mut this.file_editor;
    if editor.loading || editor.saving {
        return;
    }
    if editor
        .document
        .as_ref()
        .is_some_and(|doc| doc.input.read(cx).value().as_ref() != doc.baseline)
    {
        editor.discard = true;
        cx.notify();
        return;
    }
    editor.loading = true;
    editor.error = None;
    let roots = this.roots().to_vec();
    let open_roots = roots.clone();
    let task = cx.background_spawn(async move {
        let path = authorized(Path::new(&path), &open_roots)?;
        let text = read_text(&path)?;
        Ok::<_, io::Error>((path, text))
    });
    cx.spawn_in(window, async move |this, cx| {
        let result = task.await;
        let _ = this.update_in(cx, |this, window, cx| {
            this.file_editor.loading = false;
            match result {
                Ok((path, text)) if this.roots() == roots.as_slice() => {
                    let baseline = text.replace("\r\n", "\n");
                    let input = cx.new(|cx| {
                        EditorState::new(window, cx)
                            .language(language(&path))
                            .line_number(true)
                            .default_value(baseline.clone())
                    });
                    this.file_editor.document = Some(Document {
                        path,
                        original: text,
                        baseline,
                        input,
                    });
                }
                Ok(_) => {}
                Err(error) => {
                    this.file_editor.error =
                        Some(t!("file_editor_open_error", reason = error.to_string()).to_string())
                }
            }
            cx.notify();
        });
    })
    .detach();
    cx.notify();
}

fn save(this: &mut Observer, cx: &mut Context<Observer>) {
    let roots = this.roots().to_vec();
    let editor = &mut this.file_editor;
    let Some(doc) = &editor.document else {
        return;
    };
    if editor.loading || editor.saving {
        return;
    }
    let value = doc.input.read(cx).value().to_string();
    if value == doc.baseline {
        return;
    }
    let original = doc.original.clone();
    let path = doc.path.clone();
    let text = if original.contains("\r\n") && !original.replace("\r\n", "").contains('\n') {
        value.replace('\n', "\r\n")
    } else {
        value.clone()
    };
    editor.saving = true;
    editor.discard = false;
    editor.error = None;
    let task = cx.background_spawn(async move {
        save_text(&path, &roots, &original, &text)?;
        Ok::<_, io::Error>(text)
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        let _ = this.update(cx, |this, cx| {
            let editor = &mut this.file_editor;
            editor.saving = false;
            match result {
                Ok(text) => {
                    if let Some(doc) = &mut editor.document {
                        doc.baseline = value;
                        doc.original = text;
                    }
                }
                Err(error) => {
                    editor.error = Some(if error.kind() == io::ErrorKind::AlreadyExists {
                        t!("file_editor_conflict").to_string()
                    } else {
                        t!("file_editor_save_error", reason = error.to_string()).to_string()
                    })
                }
            }
            cx.notify();
        });
    })
    .detach();
    cx.notify();
}

pub(crate) fn panel(this: &Observer, cx: &mut Context<Observer>) -> Option<AnyElement> {
    let editor = &this.file_editor;
    if !editor.loading && editor.document.is_none() && editor.error.is_none() {
        return None;
    }
    let busy = editor.loading || editor.saving;
    let label = if editor.loading {
        t!("tree_loading").to_string()
    } else if editor.saving {
        t!("file_editor_saving").to_string()
    } else {
        editor
            .document
            .as_ref()
            .map(|doc| doc.path.display().to_string())
            .unwrap_or_default()
    };
    let muted = cx.theme().muted;
    let action = |id, label: String| {
        div()
            .id(id)
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .hover(move |s| s.bg(muted))
            .child(label)
    };
    Some(
        v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().flex_1().min_w_0().text_ellipsis().child(label))
                    .when(editor.document.is_some() && !busy, |row| {
                        row.child(
                            action("file-save", t!("file_editor_save").to_string())
                                .on_click(cx.listener(|this, _, _, cx| save(this, cx))),
                        )
                    })
                    .when(!busy, |row| {
                        row.child(
                            action(
                                "file-close",
                                if editor.discard {
                                    t!("file_editor_discard").to_string()
                                } else {
                                    t!("file_editor_close").to_string()
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let editor = &mut this.file_editor;
                                if editor.loading || editor.saving {
                                    return;
                                }
                                if !editor.discard
                                    && editor.document.as_ref().is_some_and(|doc| {
                                        doc.input.read(cx).value().as_ref() != doc.baseline
                                    })
                                {
                                    editor.discard = true;
                                } else {
                                    *editor = FileEditor::default();
                                }
                                cx.notify();
                            })),
                        )
                    }),
            )
            .when(editor.discard, |panel| {
                panel.child(div().px_2().child(t!("file_editor_unsaved").to_string()))
            })
            .when_some(editor.error.clone(), |panel, error| {
                panel.child(div().px_2().text_color(cx.theme().danger).child(error))
            })
            .when_some(editor.document.as_ref(), |panel, doc| {
                panel.child(Editor::new(&doc.input).flex_1().min_h_0().w_full())
            })
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn saves_only_authorized_text_and_preserves_external_changes() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let roots = vec![root.path().display().to_string()];
        let outside = other.path().join("a.txt");
        fs::write(&outside, "outside").unwrap();
        assert_eq!(
            authorized(&outside, &roots).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        #[cfg(unix)]
        {
            let link = root.path().join("link.txt");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(authorized(&link, &roots).is_err());
        }
        let path = root.path().join("test.rs");
        fs::write(&path, "one").unwrap();
        let path = authorized(&path, &roots).unwrap();
        assert_eq!(language(&path), "rust");
        save_text(&path, &roots, "one", "two").unwrap();
        assert_eq!(
            save_text(&path, &roots, "one", "three").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(read_text(&path).unwrap(), "two");
        fs::write(&path, b"a\0b").unwrap();
        assert_eq!(
            read_text(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
