use std::convert::Infallible;

use futures_util::stream;
use roundhouse::prelude::*;
use serde::Serialize;

routes! {
    post("/", upload, summary = "Upload a file and metadata"),
    get("/download", download, summary = "Download an attachment"),
    get("/raw", raw, summary = "Download raw bytes"),
    get("/events", events, summary = "Stream server-sent events"),
}

#[derive(Debug, roundhouse::Multipart)]
pub struct UploadForm {
    pub file: FilePart,
    #[file(max_bytes = 64 * 1024, accept = ["text/plain", "text/markdown"])]
    pub preview: Option<FilePart>,
    #[file(max_bytes = 1_000_000)]
    pub temp: Option<TempFile>,
    pub tags: Vec<String>,
    pub title: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct UploadInfo {
    pub file_name: Option<String>,
    pub file_len: usize,
    pub preview_len: Option<usize>,
    pub temp_len: Option<u64>,
    pub tags: Vec<String>,
    pub title: Option<String>,
}

#[handler]
pub async fn upload(Multipart(form): Multipart<UploadForm>) -> Json<UploadInfo> {
    let file_len = form.file.len();
    Json(UploadInfo {
        file_name: form.file.file_name,
        file_len,
        preview_len: form.preview.as_ref().map(|file| file.len()),
        temp_len: form.temp.as_ref().map(|file| file.len()),
        tags: form.tags,
        title: form.title,
    })
}

#[handler]
pub async fn download() -> Attachment<String> {
    Attachment::new("hello from uploads\n".to_string()).filename("hello.txt")
}

#[handler]
pub async fn raw() -> File {
    File::new(vec![0u8, 1, 2, 3])
}

#[handler]
pub async fn events() -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    Sse::new(stream::iter(vec![
        Ok(Event::default().event("greeting").data("hello")),
        Ok(Event::default().event("done").data("bye")),
    ]))
}
