// SPDX-License-Identifier: GPL-3.0-or-later

/// Generates a valid minimal single-page PDF with selectable text stream.
pub fn sample_text_pdf(text: &str) -> Vec<u8> {
    let stream_content = format!("BT /F1 24 Tf 100 700 Td ({}) Tj ET", text.replace('(', "\\(").replace(')', "\\)"));
    let stream_len = stream_content.len();

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let o1 = out.len();
    out.extend_from_slice(b"1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n");
    let o2 = out.len();
    out.extend_from_slice(b"2 0 obj<</Type/Pages/Count 1/Kids[3 0 R]>>endobj\n");
    let o3 = out.len();
    out.extend_from_slice(b"3 0 obj<</Type/Page/Parent 2 0 R/Resources<</Font<</F1 4 0 R>>>>/MediaBox[0 0 612 792]/Contents 5 0 R>>endobj\n");
    let o4 = out.len();
    out.extend_from_slice(b"4 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\n");
    let o5 = out.len();
    let content_obj = format!("5 0 obj<</Length {}>>stream\n{}\nendstream\nendobj\n", stream_len, stream_content);
    out.extend_from_slice(content_obj.as_bytes());

    let xref_offset = out.len();
    let xref = format!(
        "xref\n0 6\n0000000000 65535 f \n{:010} 00000 n \n{:010} 00000 n \n{:010} 00000 n \n{:010} 00000 n \n{:010} 00000 n \ntrailer<</Size 6/Root 1 0 R>>\nstartxref\n{}\n%%EOF\n",
        o1, o2, o3, o4, o5, xref_offset
    );
    out.extend_from_slice(xref.as_bytes());

    out
}

/// Generates a valid multi-page PDF.
pub fn sample_multipage_pdf(page_count: usize) -> Vec<u8> {
    let pages = page_count.max(1);
    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");

    let mut offsets = Vec::new();
    let cat_offset = out.len();
    offsets.push(cat_offset);
    out.extend_from_slice(b"1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n");

    // We will build page objects and page tree
    let pages_offset = out.len();
    offsets.push(pages_offset);

    let mut kids = String::new();
    for i in 0..pages {
        kids.push_str(&format!("{} 0 R ", 3 + i * 2));
    }
    let pages_obj = format!("2 0 obj<</Type/Pages/Count {}/Kids[{}]>>endobj\n", pages, kids.trim());
    out.extend_from_slice(pages_obj.as_bytes());

    for i in 0..pages {
        let page_num = i + 1;
        let p_obj_id = 3 + i * 2;
        let c_obj_id = 4 + i * 2;

        let p_off = out.len();
        offsets.push(p_off);
        let page_obj = format!("{} 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Contents {} 0 R>>endobj\n", p_obj_id, c_obj_id);
        out.extend_from_slice(page_obj.as_bytes());

        let stream_text = format!("BT /F1 12 Tf 50 700 Td (Page {}) Tj ET", page_num);
        let c_off = out.len();
        offsets.push(c_off);
        let contents_obj = format!("{} 0 obj<</Length {}>>stream\n{}\nendstream\nendobj\n", c_obj_id, stream_text.len(), stream_text);
        out.extend_from_slice(contents_obj.as_bytes());
    }

    let xref_offset = out.len();
    let total_objs = 2 + pages * 2;
    let mut xref = format!("xref\n0 {}\n0000000000 65535 f \n", total_objs + 1);
    for off in &offsets {
        xref.push_str(&format!("{:010} 00000 n \n", off));
    }
    xref.push_str(&format!("trailer<</Size {}/Root 1 0 R>>\nstartxref\n{}\n%%EOF\n", total_objs + 1, xref_offset));
    out.extend_from_slice(xref.as_bytes());

    out
}

pub fn sample_corrupted_pdf() -> Vec<u8> {
    let mut data = sample_text_pdf("Hermes Test");
    data.truncate(data.len() / 2); // Cut off trailer and cross-reference table
    data
}
