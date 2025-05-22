use lazy_static::lazy_static;
use mktemp::Temp;
use serde::Serialize;
use std::{io::Write, process::Command};

#[derive(Serialize)]
struct Post {
    text: String,
    embed: Option<Embed>,
}

#[derive(Serialize)]
enum Embed {
    #[serde(rename = "images")]
    Images(Vec<Image>),
    #[serde(rename = "uri")]
    Uri(String),
}

#[derive(Serialize)]
enum Image {
    #[serde(rename = "path")]
    Path(std::path::PathBuf),
    #[serde(rename = "clipboard")]
    Clipboard,
}

lazy_static! {
    static ref SOCKET: std::path::PathBuf =
        std::path::PathBuf::from("/tmp/tsky-daemon.sock");
}

fn main() {
    if !SOCKET.exists() {
        eprintln!("Is daemon running?");
        return;
    }
    let mut stream =
        std::os::unix::net::UnixStream::connect(SOCKET.as_path()).unwrap();
    eprintln!("Connected to {}", SOCKET.to_str().unwrap());

    let lines =
        from_temp_file().lines().map(str::to_string).collect::<Vec<_>>();
    let (text, images) = split_section(lines);

    let text = text.join("\n");

    let images_count = images.len();
    let images = images
        .into_iter()
        .map(|i| match i.as_str() {
            "[clipboard]" => Ok(Image::Clipboard),
            i if std::path::PathBuf::from(&i).is_file() => {
                Ok(Image::Path(std::path::PathBuf::from(&i)))
            }
            _ => Err("Invalid embed".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .drain(..std::cmp::min(4, images_count))
        .collect::<Vec<_>>();
    let uri = detect_uri(&text).unwrap();

    let post = Post {
        text,
        embed: match (images, uri) {
            (images, _) if images.len() > 0 => Some(Embed::Images(images)),
            (_, Some(uri)) => Some(Embed::Uri(uri)),
            _ => None,
        },
    };
    let post = serde_json::ser::to_string(&post).unwrap();
    stream.write_all(post.as_bytes()).unwrap();
}

fn from_temp_file() -> String {
    let temp = Temp::new_file().unwrap();
    Command::new("nvim")
        .arg(temp.to_str().unwrap())
        .spawn()
        .unwrap()
        .wait()
        .unwrap();
    return std::fs::read_to_string(temp.to_path_buf()).unwrap();
}

fn split_section(mut lines: Vec<String>) -> (Vec<String>, Vec<String>) {
    let pos = match lines.iter().position(|s| s == "---") {
        Some(i) => i,
        None => return (lines, vec![]),
    };
    let main = lines.drain(..pos).collect();
    return (main, lines[1..].to_vec());
}

// Code adopted from bsky_sdk
// I hope the original authors have a regex license
fn detect_uri(text: &String) -> Result<Option<String>, String> {
    // URL regex
    let re_url = regex::Regex::new(
        r"(?:^|\s|\()((?:https?:\/\/[\S]+)|(?:(?<domain>[a-z][a-z0-9]*(?:\.[a-z0-9]+)+)[\S]*))",
    )
    .map_err(|_| "invalid regex".to_string())?;
    let Some(capture) = re_url.captures(&text) else {
        return Ok(None);
    };

    let m = capture.get(1).ok_or("invalid capture".to_string())?;
    let mut uri = if let Some(domain) = capture.name("domain") {
        if !psl::suffix(domain.as_str().as_bytes())
            .map_or(false, |suffix| suffix.is_known())
        {
            return Err("Unknown domain suffix".to_string());
        }
        format!("https://{}", m.as_str())
    } else {
        m.as_str().into()
    };

    // ending punctuation regex
    let re_ep = regex::Regex::new(r"[.,;:!?]$")
        .map_err(|_| "invalid regex".to_string())?;
    if re_ep.is_match(&uri) || (uri.ends_with(')') && !uri.contains('(')) {
        uri.pop();
    }

    return Ok(Some(uri));
}
