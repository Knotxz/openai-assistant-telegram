use async_openai::types::{CreateMessageRequestArgs, CreateRunRequestArgs, CreateThreadRequestArgs, MessageContent, RunStatus};
use flowsnet_platform_sdk::logger;
use reqwest::header::{HeaderMap, HeaderValue};
use tg_flows::{listen_to_update, update_handler, Telegram, UpdateKind};
use serde::Deserialize;

#[derive(Deserialize)]
struct ThreadResponse {
    id: String,
    // Add other fields as necessary based on the API response
}

#[no_mangle]
#[tokio::main(flavor = "current_thread")]
pub async fn on_deploy() {
    logger::init();

    let telegram_token = std::env::var("telegram_token").unwrap();
    listen_to_update(telegram_token).await;
}

#[update_handler]
async fn handler(update: tg_flows::Update) {
    logger::init();
    let telegram_token = std::env::var("telegram_token").unwrap();
    let tele = Telegram::new(telegram_token);

    if let UpdateKind::Message(msg) = update.kind {
        let text = msg.text().unwrap_or("");
        let chat_id = msg.chat.id;

        let thread_id = match store_flows::get(chat_id.to_string().as_str()) {
            Some(ti) => match text == "/restart" {
                true => {
                    delete_thread(ti.as_str().unwrap()).await;
                    store_flows::del(chat_id.to_string().as_str());
                    return;
                }
                false => ti.as_str().unwrap().to_owned(),
            },
            None => {
                let ti = create_thread().await;
                store_flows::set(
                    chat_id.to_string().as_str(),
                    serde_json::Value::String(ti.clone()),
                    None,
                );
                ti
            }
        };

        let response = run_message(thread_id.as_str(), String::from(text)).await;
        _ = tele.send_message(chat_id, response);
    }
}

async fn create_thread() -> String {
    let reqwest_client = reqwest::Client::new();
    let url = "https://api.openai.com/v1/threads"; // Adjust the URL as needed

    let create_thread_request = CreateThreadRequestArgs::default().build().unwrap();

    let mut headers = HeaderMap::new();
    headers.insert("OpenAI-Beta", HeaderValue::from_static("assistants=v2"));
    headers.insert("Authorization", HeaderValue::from_str(&format!("Bearer {}", std::env::var("OPENAI_API_KEY").unwrap())).unwrap());

    let response = reqwest_client
        .post(url)
        .headers(headers)
        .json(&create_thread_request)
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        let thread: ThreadResponse = response.json().await.unwrap();
        log::info!("New thread (ID: {}) created.", thread.id);
        thread.id
    } else {
        panic!("Failed to create thread. {:?}", response.text().await.unwrap());
    }
}

async fn delete_thread(thread_id: &str) {
    let reqwest_client = reqwest::Client::new();
    let url = format!("https://api.openai.com/v1/threads/{}", thread_id);

    let mut headers = HeaderMap::new();
    headers.insert("OpenAI-Beta", HeaderValue::from_static("assistants=v2"));
    headers.insert("Authorization", HeaderValue::from_str(&format!("Bearer {}", std::env::var("OPENAI_API_KEY").unwrap())).unwrap());

    let response = reqwest_client
        .delete(&url)
        .headers(headers)
        .send()
        .await
        .unwrap();

    if response.status().is_success() {
        log::info!("Old thread (ID: {}) deleted.", thread_id);
    } else {
        log::error!("Failed to delete thread. {:?}", response.text().await.unwrap());
    }
}

async fn run_message(thread_id: &str, text: String) -> String {
    let reqwest_client = reqwest::Client::new();
    let url = format!("https://api.openai.com/v1/threads/{}/messages", thread_id);
    let assistant_id = std::env::var("ASSISTANT_ID").unwrap();

    let mut create_message_request = CreateMessageRequestArgs::default().build().unwrap();
    create_message_request.content = text;

    let mut headers = HeaderMap::new();
    headers.insert("OpenAI-Beta", HeaderValue::from_static("assistants=v2"));
    headers.insert("Authorization", HeaderValue::from_str(&format!("Bearer {}", std::env::var("OPENAI_API_KEY").unwrap())).unwrap());

    let _ = reqwest_client
        .post(&url)
        .headers(headers.clone())
        .json(&create_message_request)
        .send()
        .await
        .unwrap();

    let run_url = format!("https://api.openai.com/v1/threads/{}/runs", thread_id);
    let mut create_run_request = CreateRunRequestArgs::default().build().unwrap();
    create_run_request.assistant_id = assistant_id;

    let run_response = reqwest_client
        .post(&run_url)
        .headers(headers.clone())
        .json(&create_run_request)
        .send()
        .await
        .unwrap();

    // Store the JSON response in a variable
    let run_response_json: serde_json::Value = run_response.json().await.unwrap();
    let run_id = run_response_json["id"].as_str().unwrap(); // Access the run_id from the stored JSON

    // Poll for the run status
    let mut result = Some("Timeout");
    for _ in 0..5 {
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let run_status_url = format!("https://api.openai.com/v1/threads/{}/runs/{}", thread_id, run_id);
        let run_object = reqwest_client
            .get(&run_status_url)
            .headers(headers.clone())
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();

        result = match run_object["status"].as_str().unwrap() {
            "queued" | "in_progress" | "cancelling" => {
                continue;
            }
            "requires_action" => Some("Action required for OpenAI assistant"),
            "cancelled" => Some("Run is cancelled"),
            "failed" => Some("Run is failed"),
            "expired" => Some("Run is expired"),
            "completed" => None,
            _ => Some("Unknown status"),
        };
        break;
    }

    match result {
        Some(r) => String::from(r),
        None => {
            // Retrieve the last message from the thread
            let messages_url = format!("https://api.openai.com/v1/threads/{}/messages", thread_id);
            let thread_messages = reqwest_client
                .get(&messages_url)
                .headers(headers)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();

            let c = thread_messages["data"].as_array().unwrap().last().unwrap();
            if let Some(content) = c.get("content") {
                if let Some(text_content) = content.as_str() {
                    return text_content.to_string();
                }
            }
            return String::from("No messages found.");
        }
    }
}
