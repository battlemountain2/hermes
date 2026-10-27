use super::*;

#[test]
fn isolated_profile_disables_macros_active_content_and_link_updates() {
    assert!(PROFILE.contains("DisableMacrosExecution"));
    assert!(PROFILE.contains("DisableActiveContent"));
    assert!(PROFILE.contains("BlockUntrustedRefererLinks"));
    assert!(PROFILE.contains("<value>3</value>"));
    assert!(!PROFILE.contains("UserInstallation"));
    assert!(PROFILE.contains("/org.openoffice.Office.Writer/Content/Update"));
    assert!(PROFILE.contains("/org.openoffice.Office.Calc/Content/Update"));
}

#[test]
fn office_renderer_discovery_never_uses_the_process_path_or_user_profile() {
    if let Some(path) = renderer() {
        assert!(path.starts_with("/usr") || path.starts_with("/opt/libreoffice"));
        assert!(path.is_absolute());
    }
}

#[test]
fn icon_exports_only_request_the_first_page_with_bounded_image_resolution() {
    for filter in ["writer_pdf_Export", "impress_pdf_Export", "calc_pdf_Export"] {
        let full = pdf_filter_options(filter, false);
        let full: serde_json::Value =
            serde_json::from_str(full.splitn(3, ':').nth(2).unwrap()).unwrap();
        assert!(full.get("PageRange").is_none());
        assert_eq!(full["ReduceImageResolution"]["value"], "true");
        assert_eq!(full["MaxImageResolution"]["value"], "150");
        let options = pdf_filter_options(filter, true);
        let options: serde_json::Value =
            serde_json::from_str(options.splitn(3, ':').nth(2).unwrap()).unwrap();
        assert_eq!(options["PageRange"]["value"], "1");
        assert_eq!(options["ReduceImageResolution"]["value"], "true");
        assert_eq!(options["MaxImageResolution"]["value"], "75");
    }
}

#[test]
fn embedded_office_images_are_scaled_and_unsafe_headers_rejected_before_decode() {
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 400, 200).unwrap();
    pixels.fill(0x234567ff);
    let bytes = pixels.save_to_bufferv("png", &[]).unwrap();
    let png = embedded_thumbnail(&bytes, 96).unwrap();
    let size = crate::sandbox_helper::sniff::sniff_png_dimensions(&png).unwrap();
    assert_eq!((size.width, size.height), (96, 48));
    assert!(embedded_thumbnail(b"Not an image", 96).is_err());
    let mut huge = bytes;
    huge[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(
        embedded_thumbnail(&huge, 96)
            .unwrap_err()
            .contains("budget")
    );
}
