use std::io::Read;

use bsky_sdk::{
    agent::config::{Config, FileStore},
    rich_text::RichText,
    BskyAgent,
};
use dotenvy::dotenv;
use lazy_static::lazy_static;
use serde::Deserialize;

lazy_static! {
    static ref SESSION_FILE: String = {
        let home = std::env::var("HOME").unwrap();
        format!("{}/.local/share/tsky/session.json", home)
    };
    static ref SOCKET: std::path::PathBuf =
        std::path::PathBuf::from("/tmp/tsky-daemon.sock");
}

#[derive(Debug, Deserialize)]
struct Post {
    text: String,
    embed: Option<Embed>,
}

impl Post {
    async fn post(self, agent: BskyAgent) -> Result<String, String> {
        let created_at = atrium_api::types::string::Datetime::now();
        let facets = match RichText::new_with_detect_facets(&self.text).await {
            Ok(richtext) => richtext.facets,
            Err(e) => {
                return Err(format!("Cannot parse richtext: {}", e));
            }
        };
        let embed = match self.embed {
            Some(e) => Some(e.to_record_embed(agent.clone()).await?),
            None => None,
        };
        let r = agent
            .create_record(atrium_api::app::bsky::feed::post::RecordData {
                created_at,
                embed,
                entities: None,
                facets,
                labels: None,
                langs: None,
                reply: None,
                tags: None,
                text: self.text,
            })
            .await;
        return r.map(|o| o.uri.clone()).map_err(|e| e.to_string());
    }
}

#[derive(Debug, Deserialize)]
enum Embed {
    #[serde(rename = "images")]
    Images(Vec<Image>),
    #[serde(rename = "uri")]
    Uri(String),
}

impl Embed {
    async fn to_record_embed(
        self,
        agent: BskyAgent,
    ) -> Result<
        atrium_api::types::Union<
            atrium_api::app::bsky::feed::post::RecordEmbedRefs,
        >,
        String,
    > {
        use atrium_api::{
            app::bsky::feed::post::RecordEmbedRefs, types::Union,
        };
        match self {
            Embed::Images(images) => {
                let promises =
                    images.into_iter().map(|i| i.upload(agent.clone()));
                let images = atrium_api::app::bsky::embed::images::MainData {
                    images: futures::future::try_join_all(promises).await?,
                }
                .into();
                return Ok(Union::Refs(
                    RecordEmbedRefs::AppBskyEmbedImagesMain(Box::new(images)),
                ));
            }
            Embed::Uri(uri) => {
                let external = fetch_uri(uri, agent).await?;
                return Ok(Union::Refs(
                    RecordEmbedRefs::AppBskyEmbedExternalMain(Box::new(
                        external,
                    )),
                ));
            }
        }
    }
}

#[derive(Debug, Deserialize)]
enum Image {
    #[serde(rename = "path")]
    Path(std::path::PathBuf),
    #[serde(rename = "clipboard")]
    Clipboard,
}

impl Image {
    async fn upload(
        self,
        agent: BskyAgent,
    ) -> Result<atrium_api::app::bsky::embed::images::Image, String> {
        let blob = match self {
            Image::Path(path) => blob_from_path(path)?,
            Image::Clipboard => blob_from_clipboard()?,
        };
        let size = imagesize::blob_size(&blob).map_err(|e| e.to_string())?;
        let ar = Some(
            atrium_api::app::bsky::embed::defs::AspectRatioData {
                height: (size.height as u64).try_into().unwrap(),
                width: (size.width as u64).try_into().unwrap(),
            }
            .into(),
        );
        eprintln!("Uploading image");
        let blob = agent
            .api
            .com
            .atproto
            .repo
            .upload_blob(blob)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(atrium_api::app::bsky::embed::images::ImageData {
            alt: String::new(),
            aspect_ratio: ar,
            image: blob.data.blob,
        }
        .into());
    }
}

async fn fetch_uri(
    uri: String,
    agent: BskyAgent,
) -> Result<atrium_api::app::bsky::embed::external::Main, String> {
    let text = reqwest::get(uri.clone())
        .await
        .map_err(|e| format!("Cannot fetch page: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Cannot fetch text: {}", e))?;
    let (description, title, thumb) = {
        let dom = tl::parse(&text, tl::ParserOptions::default()).unwrap();
        let parser = dom.parser();
        let meta = dom
            .query_selector("meta")
            .unwrap()
            .filter_map(|h| h.get(parser))
            .filter_map(|t| {
                let attributes = t.as_tag()?.attributes();
                let property = attributes.get("property")??.as_bytes();
                let property = std::str::from_utf8(property).ok()?;
                if !property.starts_with("og:") {
                    return None;
                }
                let content = attributes.get("content")??.as_bytes();
                let content = std::str::from_utf8(content).ok()?;
                Some((property, content))
            })
            .collect::<Vec<_>>();
        let description = meta
            .iter()
            .find(|(p, _)| *p == "og:description")
            .map(|(_, c)| c.to_string())
            .unwrap_or(String::new());
        let title = meta
            .iter()
            .find(|(p, _)| *p == "og:title")
            .map(|(_, c)| c.to_string())
            .unwrap_or(String::new());
        let thumb = meta
            .iter()
            .find(|(p, _)| *p == "og:image")
            .map(|(_, c)| c.to_string());
        (description, title, thumb)
    };
    let thumb = if let Some(thumb) = thumb {
        eprintln!("Fetching thumbnail");
        let Ok(res) = reqwest::get(thumb).await else {
            return Err("Cannot fetch image".to_string());
        };
        let Ok(blob) = res.bytes().await else {
            return Err("Cannot fetch blob".to_string());
        };

        eprintln!("Uploading thumbnail");
        let r#ref = agent.api.com.atproto.repo.upload_blob(blob.to_vec());
        let blob = match r#ref.await {
            Ok(r) => r,
            Err(e) => {
                return Err(format!("Cannot upload thumbnail: {}", e));
            }
        };
        Some(blob.blob.clone())
    } else {
        None
    };

    let external = atrium_api::app::bsky::embed::external::ExternalData {
        description,
        title,
        uri,
        thumb,
    }
    .into();
    return Ok(
        atrium_api::app::bsky::embed::external::MainData { external }.into()
    );
}

fn blob_from_path(path: std::path::PathBuf) -> Result<Vec<u8>, String> {
    let accepted_types = ["image/jpeg", "image/png", "image/webp", "image/bmp"];

    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("Cannot open file: {}", e))?;
    let mut data = vec![];
    file.read_to_end(&mut data)
        .map_err(|e| format!("Cannot read from file: {}", e))?;

    let mime = tree_magic_mini::from_u8(&data);
    if !accepted_types.contains(&mime) {
        return Err("Filetype not supported".to_string());
    };

    return Ok(data);
}

fn blob_from_clipboard() -> Result<Vec<u8>, String> {
    let mime_types = wl_clipboard_rs::paste::get_mime_types(
        wl_clipboard_rs::paste::ClipboardType::Regular,
        wl_clipboard_rs::paste::Seat::Unspecified,
    )
    .map_err(|e| format!("Cannot get clipboard mime type: {}", e))?;

    let accepted_types = ["image/jpeg", "image/png", "image/webp", "image/bmp"];
    let mime = accepted_types
        .iter()
        .find(|t| mime_types.contains(**t))
        .ok_or_else(|| "No supported images found in clipboard".to_string())?;

    let content = wl_clipboard_rs::paste::get_contents(
        wl_clipboard_rs::paste::ClipboardType::Regular,
        wl_clipboard_rs::paste::Seat::Unspecified,
        wl_clipboard_rs::paste::MimeType::Specific(mime),
    );
    match content {
        Ok((mut pipe, _)) => {
            let mut data = vec![];
            pipe.read_to_end(&mut data)
                .map_err(|e| format!("Cannot read from clipboard: {}", e))?;
            return Ok(data);
        }
        Err(wl_clipboard_rs::paste::Error::NoSeats)
        | Err(wl_clipboard_rs::paste::Error::ClipboardEmpty)
        | Err(wl_clipboard_rs::paste::Error::NoMimeType) => {
            return Err("Empty clipboard".to_string())
        }
        Err(e) => {
            return Err(format!("Cannot paste from clipboard: {}", e));
        }
    }
}

#[tokio::main]
async fn main() {
    eprintln!("Logging in");
    let agent = login().await;
    eprintln!(
        "Logged in as {}",
        agent.get_session().await.unwrap().handle.to_string()
    );

    if SOCKET.exists() {
        std::fs::remove_file(SOCKET.as_path()).unwrap();
    }
    let listener =
        std::os::unix::net::UnixListener::bind(SOCKET.as_path()).unwrap();
    eprintln!("Listening at /tmp/tsky-daemon.sock");
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Cannot read stream: {}", e);
                continue;
            }
        };
        let bytes = stream.bytes();
        let input = bytes
            .take_while(|c| c.as_ref().is_ok_and(|c| *c != 0))
            .collect::<Result<Vec<u8>, std::io::Error>>()
            .unwrap();
        let input = String::from_utf8(input).unwrap();
        let post: Post = serde_json::from_str(&input).unwrap();
        eprintln!("{:?}", post);
        eprintln!("Posting");
        match post.post(agent.clone()).await {
            Ok(uri) => {
                eprintln!("Posted: {}", uri);
            }
            Err(e) => {
                eprintln!("Cannot post: {}", e);
            }
        }
    }
}

async fn login() -> BskyAgent {
    match Config::load(&FileStore::new(SESSION_FILE.as_str())).await {
        Ok(config) => {
            let agent = BskyAgent::builder()
                .config(config)
                .build()
                .await
                .expect("Cannot create bsky agent from session file");
            return agent;
        }
        Err(e) => {
            eprintln!(
                "Cannot load session file {}: {}\r",
                SESSION_FILE.as_str(),
                e
            );
            eprintln!("Using environment variables to login\r");

            dotenv().unwrap_or_else(|e| {
                eprintln!("Cannot load .env: {}\r", e);
                std::path::PathBuf::new()
            });

            let handle = std::env::var("handle").expect("Cannot get $handle");
            let password =
                std::env::var("password").expect("Cannot get $password");

            let agent = BskyAgent::builder()
                .build()
                .await
                .expect("Cannot create bsky agent");
            agent.login(handle, password).await.expect("Cannot login to bsky");

            let path = std::path::PathBuf::from(SESSION_FILE.as_str());
            let dir = path.parent().unwrap();
            if !dir.exists() {
                std::fs::create_dir_all(dir).expect(
                    format!(
                        "Cannot create directory {}",
                        dir.to_str().unwrap()
                    )
                    .as_str(),
                );
            }
            agent
                .to_config()
                .await
                .save(&FileStore::new(SESSION_FILE.as_str()))
                .await
                .expect(
                    format!(
                        "Cannot save session file {}",
                        SESSION_FILE.as_str()
                    )
                    .as_str(),
                );
            return agent;
        }
    };
}
