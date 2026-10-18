// SPDX-License-Identifier: GPL-3.0-only

use std::sync::LazyLock;

use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::{DefaultLocalizer, DesktopLanguageRequester, LanguageLoader, Localizer};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

pub static LANGUAGE_LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    // No Unicode isolation marks around arguments: they end up in shelf names.
    loader.set_use_isolating(false);
    loader
        .load_fallback_language(&Localizations)
        .expect("fallback language must load");
    loader
});

#[macro_export]
macro_rules! fl {
    ($message_id:literal) => {{
        i18n_embed_fl::fl!($crate::i18n::LANGUAGE_LOADER, $message_id)
    }};
    ($message_id:literal, $($args:expr),*) => {{
        i18n_embed_fl::fl!($crate::i18n::LANGUAGE_LOADER, $message_id, $($args), *)
    }};
}

/// Selects the translations matching the desktop language.
pub fn init() {
    let localizer = DefaultLocalizer::new(&*LANGUAGE_LOADER, &Localizations);
    if let Err(err) = localizer.select(&DesktopLanguageRequester::requested_languages()) {
        tracing::warn!("failed to load translations: {err}");
    }
}
