use std::io::Read;

use bsky_sdk::{
    agent::config::{Config, FileStore},
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
}

#[derive(Debug, Deserialize)]
struct Post {
    content: String,
    attachments: Option<std::path::PathBuf>, // TODO: make it enum of possible attachments
}

#[tokio::main]
async fn main() {
    eprintln!("Logging in");
    let agent = login().await;

    loop {
        let stdin = std::io::stdin();
        let bytes = stdin.bytes();
        let input = bytes
            .take_while(|c| c.as_ref().is_ok_and(|c| *c != 0))
            .collect::<Result<Vec<u8>, std::io::Error>>()
            .unwrap();
        let input = String::from_utf8(input).unwrap();
        let post: Post = serde_json::from_str(&input).unwrap();
        println!("{:?}", post);

        // TODO: post the thing
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
