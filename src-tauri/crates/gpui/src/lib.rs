//! Workflow window model and transport, independent of the GUI backend.
//!
//! Reads snapshots and live events and carries the user's actions — messages,
//! approvals and stops — back to the runtime.
pub mod chimes;
#[cfg(feature = "gui")]
pub mod components;
pub mod icons;
pub mod metrics;
pub mod model;
pub mod native;
pub mod prefs;
pub mod presentation;
pub mod terminal;
pub mod types;
pub mod worker;

rust_i18n::i18n!("locales", fallback = "en");

#[cfg(test)]
mod locale_tests {
    #[test]
    fn observer_labels_resolve_in_every_interface_locale() {
        assert_eq!(
            rust_i18n::t!("sessions", locale = "en"),
            "Workflow sessions"
        );
        assert_eq!(rust_i18n::t!("sessions", locale = "zh-Hans"), "工作流会话");
        assert_eq!(rust_i18n::t!("sessions", locale = "zh-Hant"), "工作流會話");
        assert_ne!(
            rust_i18n::t!("metrics_error", locale = "en"),
            "metrics_error"
        );
    }

    #[test]
    fn file_changes_title_omits_zero_counts_in_every_locale() {
        for (locale, all, only_modified, added_and_deleted) in [
            (
                "en",
                "File changes (Added x2 · Modified x3 · Deleted x1)",
                "File changes (Modified x3)",
                "File changes (Added x2 · Deleted x1)",
            ),
            (
                "zh-Hans",
                "文件修改列表（新增 x2 · 修改 x3 · 删除 x1）",
                "文件修改列表（修改 x3）",
                "文件修改列表（新增 x2 · 删除 x1）",
            ),
            (
                "zh-Hant",
                "檔案修改清單（新增 x2 · 修改 x3 · 刪除 x1）",
                "檔案修改清單（修改 x3）",
                "檔案修改清單（新增 x2 · 刪除 x1）",
            ),
        ] {
            use crate::presentation::file_changes_title;
            assert_eq!(file_changes_title([2, 3, 1], locale), all);
            assert_eq!(file_changes_title([0, 3, 0], locale), only_modified);
            assert_eq!(file_changes_title([2, 0, 1], locale), added_and_deleted);
        }
    }

    #[test]
    fn status_placeholders_use_rust_i18n_syntax() {
        let locales = [
            include_str!("../locales/en.yml"),
            include_str!("../locales/zh-Hans.yml"),
            include_str!("../locales/zh-Hant.yml"),
        ];
        let keys = |text: &'static str| {
            text.lines()
                .filter_map(|line| line.split_once(':').map(|(key, _)| key))
                .collect::<Vec<_>>()
        };
        for locale in locales {
            assert!(
                !locale.contains("{{"),
                "vue-style placeholder left in locale"
            );
            assert_eq!(keys(locale), keys(locales[0]));
        }
        assert_eq!(
            rust_i18n::t!(
                "live_elapsed",
                locale = "zh-Hans",
                text = "运行",
                seconds = 3
            ),
            "运行 ...(3秒)"
        );
    }
}
