const FOXIT: &str = include_str!("../docs/third-party/pdf-assets/pdfium-foxit.txt");
const ADOBE: &str = include_str!("../docs/third-party/pdf-assets/adobe-cmap.txt");
const ICC: &str = include_str!("../docs/third-party/pdf-assets/icc-cc0.txt");
const ALLOC: &str = include_str!("../docs/third-party/pdf-assets/alloc-stdlib.txt");

const SECTIONS: [(&str, &str); 4] = [
    (
        "PDF 備用字型：PDFium／Foxit（hayro-font 0.5.0）\nOriginal code copyright 2014 Foxit Software Inc. http://www.foxitsoftware.com",
        FOXIT,
    ),
    (
        "PDF 字元對映：Adobe CMaps（hayro-cmap 0.1.1）",
        ADOBE,
    ),
    (
        "PDF 色彩描述檔：CGATS001、LAB、ProPhoto（hayro-interpret 0.9.0／hayro-jpeg2000 0.5.0）\nCGATS001 與 ProPhoto 由 Compact ICC Profiles 提供；LAB 由 LCMS 產生。兩個 Hayro 套件發布相同的 CC0 通知，下列全文列一次。",
        ICC,
    ),
    (
        "PDF 處理依賴：alloc-stdlib 0.2.4（上游發布來源的 BSD 通知）",
        ALLOC,
    ),
];

pub(crate) fn body() -> String {
    let mut body = concat!(
        "紙間 ", env!("CARGO_PKG_VERSION"), "\n\n",
        "第三方補充通知（PDF 資產及其依賴）\n",
        "此頁僅列下列項目，不是完整的依賴或 Windows 執行階段授權清單。\n"
    ).to_owned();
    for (heading, text) in SECTIONS {
        body.push('\n');
        body.push_str(heading);
        body.push_str("\n\n");
        body.push_str(text);
        if !text.ends_with('\n') {
            body.push('\n');
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_component_versions_exist_in_the_lock() {
        let lock = include_str!("../Cargo.lock").replace("\r\n", "\n");
        for (name, version) in [
            ("hayro-font", "0.5.0"),
            ("hayro-cmap", "0.1.1"),
            ("hayro-interpret", "0.9.0"),
            ("hayro-jpeg2000", "0.5.0"),
            ("alloc-stdlib", "0.2.4"),
        ] {
            assert!(
                lock.contains(&format!("\nname = \"{name}\"\nversion = \"{version}\"\n")),
                "PDF notice version changed; review {name} {version}"
            );
        }
    }

    #[test]
    fn body_shows_reviewed_pdf_notices_once_in_order_without_markup() {
        let rendered = body();
        assert!(rendered.contains("不是完整的依賴或 Windows 執行階段授權清單"));
        assert!(!rendered.contains('\0'));
        let mut prior = 0;
        for (heading, notice) in SECTIONS {
            assert_eq!(rendered.matches(notice).count(), 1, "{heading}");
            let label = rendered.find(heading).unwrap();
            let text = rendered.find(notice).unwrap();
            assert!(prior <= label && label < text, "{heading}");
            prior = text + notice.len();
        }
    }
}
