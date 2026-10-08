//! Startup work done on a helper thread while GPUI initialises its platform (ADR 0110).
//!
//! Before any of our code runs, GPUI's Windows platform creates its Direct3D 11 device on the
//! main thread (~300 ms on this laptop's discrete GPU) and then loads DirectWrite's system font
//! collection, asking it to check for newly installed fonts (~160 ms). DirectWrite's shared
//! factory and its font collection are per process, so loading the collection on another
//! thread while the device is being created leaves GPUI's own request only the update check
//! (~50 ms).

#![allow(unsafe_code)]

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory3,
};

/// Loads DirectWrite's system font collection on a helper thread. Call it right before
/// `gpui::Application::new`; nothing waits for it, and if it fails GPUI loads the fonts itself
/// as before.
pub fn load_fonts() {
    let spawned = std::thread::Builder::new()
        .name("load fonts".into())
        .spawn(|| {
            // SAFETY: plain calls into DirectWrite, which needs no COM initialisation and whose
            // factory and font objects may be used from any thread. The out pointer refers to
            // a local that outlives the call; both objects are reference counted and released
            // on drop.
            let loaded = unsafe {
                DWriteCreateFactory::<IDWriteFactory3>(DWRITE_FACTORY_TYPE_SHARED).and_then(
                    |factory| {
                        let mut collection = None;
                        // The request GPUI makes: no downloadable fonts, check for updates.
                        factory.GetSystemFontCollection(false, &mut collection, true)
                    },
                )
            };
            if let Err(error) = loaded {
                tracing::debug!(%error, "could not load fonts ahead of GPUI");
            }
        });
    if let Err(error) = spawned {
        tracing::debug!(%error, "could not start loading fonts ahead of GPUI");
    }
}
