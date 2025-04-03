async fn run_message(thread_id: &str, text: String) -> String {
    let client = Client::new();
    let assistant_id = std::env::var("ASSISTANT_ID").unwrap();

    let mut create_message_request = CreateMessageRequestArgs::default().build().unwrap();
    create_message_request.content = text;

    // Create a new header map and insert the required headers
    let mut headers = HeaderMap::new();
    headers.insert("OpenAI-Beta", HeaderValue::from_static("assistants=v2"));
    headers.insert("Authorization", HeaderValue::from_str(&format!("Bearer {}", std::env::var("OPENAI_API_KEY").unwrap())).unwrap());

    // Send the message
    client
        .threads()
        .messages(thread_id)
        .create(create_message_request)
        .await
        .unwrap();

    let mut create_run_request = CreateRunRequestArgs::default().build().unwrap();
    create_run_request.assistant_id = assistant_id;

    let run_response = client
        .threads()
        .runs(thread_id)
        .create(create_run_request)
        .await
        .unwrap();

    let run_id = run_response.id;

    let mut result = Some("Timeout");
    for _ in 0..5 {
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let run_object = client
            .threads()
            .runs(thread_id)
            .retrieve(run_id.as_str())
            .await
            .unwrap();
        result = match run_object.status {
            RunStatus::Queued | RunStatus::InProgress | RunStatus::Cancelling => {
                continue;
            }
            RunStatus::RequiresAction => Some("Action required for OpenAI assistant"),
            RunStatus::Cancelled => Some("Run is cancelled"),
            RunStatus::Failed => Some("Run is failed"),
            RunStatus::Expired => Some("Run is expired"),
            RunStatus::Completed => None,
        };
        break;
    }

    match result {
        Some(r) => String::from(r),
        None => {
            // Retrieve the last message from the thread
            let messages_url = format!("https://api.openai.com/v1/threads/{}/messages", thread_id);
            let thread_messages = client
                .get(&messages_url)
                .headers(headers) // Use the same headers
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();

            // Access the messages array
            let messages = thread_messages["data"].as_array().unwrap();
            if let Some(last_message) = messages.last() {
                if let Some(content) = last_message.get("content") {
                    if let Some(text_content) = content.get("text") {
                        return text_content["value"].as_str().unwrap().to_string();
                    }
                }
            }
            return String::from("No messages found.");
        }
    }
}
