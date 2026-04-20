pub(crate) const OUTER_HTML: &str = r###"{let rv = ''; if(document.doctype){rv+=new XMLSerializer().serializeToString(document.doctype);} if(document.documentElement){rv+=document.documentElement.outerHTML;} rv}"###;
/// UTF-16 code-unit length of the same serialization as [`OUTER_HTML`].
/// Returns a primitive `Number` so the result carries no `RemoteObjectId`
/// and no runtime release is required.
pub(crate) const OUTER_HTML_LEN: &str = r###"{let rv = ''; if(document.doctype){rv+=new XMLSerializer().serializeToString(document.doctype);} if(document.documentElement){rv+=document.documentElement.outerHTML;} rv.length}"###;
/// UTF-8 byte length of the same serialization as [`OUTER_HTML`], computed
/// inside V8 via `Blob` so the bytes are never shipped to Rust.
pub(crate) const OUTER_HTML_BYTE_LEN: &str = r###"{let rv = ''; if(document.doctype){rv+=new XMLSerializer().serializeToString(document.doctype);} if(document.documentElement){rv+=document.documentElement.outerHTML;} new Blob([rv]).size}"###;
/// XML serializer for custom pages or testing.
pub(crate) const FULL_XML_SERIALIZER_JS: &str = "(()=>{let e=document.querySelector('#webkit-xml-viewer-source-xml');let x=e?e.innerHTML:new XMLSerializer().serializeToString(document);return x.startsWith('<?xml')?x:'<?xml version=\"1.0\" encoding=\"UTF-8\"?>\\n'+x})()";

/// Generate a marker at the click position. Useful for older chrome versions.
pub(crate) fn generate_marker_js(x: f64, y: f64) -> String {
    format!(
        "(()=>{{const m=document.createElement('div');m.style='position:absolute;left:{}px;top:{}px;width:10px;height:10px;background:hsl('+Math.floor(Math.random()*360)+',100%,50%);border:2px solid white;border-radius:50%;z-index:9999;pointer-events:none';document.body.appendChild(m);}})();",
        x - 5.0,
        y - 5.0
    )
}
