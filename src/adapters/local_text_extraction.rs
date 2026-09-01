// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

use crate::{
    sandbox::{Cancellation, ParseOperation},
    services::{TextExtractionProvider, TextExtractor},
};

#[derive(Default)]
pub struct LocalTextExtractionProvider;

impl TextExtractionProvider for LocalTextExtractionProvider {
    fn extract_text(
        &self,
        path: &Path,
        extractor: TextExtractor,
        byte_limit: usize,
        cancelled: Arc<AtomicBool>,
    ) -> Result<String, String> {
        let operation = match extractor {
            TextExtractor::Pdf => ParseOperation::ExtractPdfText,
            TextExtractor::PlainText => {
                return Err("Direct text extraction does not require the sandbox".to_owned());
            }
        };
        let limit = i32::try_from(byte_limit).unwrap_or(i32::MAX);
        let output = crate::sandbox::parse(
            path,
            operation,
            limit,
            &Cancellation::from_shared(cancelled),
        )
        .map_err(|message| format!("Some PDF contents could not be searched: {message}"))?;
        String::from_utf8(output.data)
            .map_err(|_| "The PDF text extractor returned invalid UTF-8".to_owned())
    }
}
