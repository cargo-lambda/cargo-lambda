use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use axum::{body::Body, http::response::Builder, response::Response};
use bytes::{Bytes, BytesMut};
use http_body_util::BodyExt;
use serde::Serialize;

use crate::error::ServerError;

/// Encodes a chunk of data as an EventStream PayloadChunk event
pub fn encode_payload_chunk(chunk_data: Bytes) -> Result<Bytes, ServerError> {
    let message = Message::new(chunk_data)
        .add_header(Header::new(
            ":event-type",
            HeaderValue::String("PayloadChunk".into()),
        ))
        .add_header(Header::new(
            ":content-type",
            HeaderValue::String("application/octet-stream".into()),
        ));

    let mut buf = BytesMut::new();
    write_message_to(&message, &mut buf).map_err(ServerError::EventStreamEncodingError)?;

    Ok(buf.freeze())
}

/// Encodes an InvokeComplete event with optional error information
pub fn encode_invoke_complete(
    error_code: Option<String>,
    error_details: Option<String>,
) -> Result<Bytes, ServerError> {
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct InvokeCompletePayload {
        #[serde(skip_serializing_if = "Option::is_none")]
        error_code: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_details: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        log_result: Option<String>,
    }

    let payload = InvokeCompletePayload {
        error_code,
        error_details,
        log_result: None,
    };

    let payload_json = serde_json::to_vec(&payload).map_err(ServerError::SerializationError)?;

    let message = Message::new(Bytes::from(payload_json))
        .add_header(Header::new(
            ":event-type",
            HeaderValue::String("InvokeComplete".into()),
        ))
        .add_header(Header::new(
            ":content-type",
            HeaderValue::String("application/json".into()),
        ));

    let mut buf = BytesMut::new();
    write_message_to(&message, &mut buf).map_err(ServerError::EventStreamEncodingError)?;

    Ok(buf.freeze())
}

/// Transforms a Lambda streaming response into an EventStream response
pub async fn create_eventstream_response(
    builder: Builder,
    body: &mut Body,
) -> Result<Response<Body>, ServerError> {
    // Collect all frames from the body
    let mut eventstream_chunks = Vec::new();

    // Process each chunk and convert to EventStream PayloadChunk events
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(ServerError::DataDeserialization)?;

        if let Ok(data) = frame.into_data() {
            if !data.is_empty() {
                let eventstream_chunk = encode_payload_chunk(data)?;
                eventstream_chunks.push(eventstream_chunk);
            }
        }
    }

    // Add InvokeComplete event at the end
    let invoke_complete = encode_invoke_complete(None, None)?;
    eventstream_chunks.push(invoke_complete);

    // Combine all chunks into a single body
    let combined_body = eventstream_chunks
        .into_iter()
        .flat_map(|chunk| chunk.to_vec())
        .collect::<Vec<u8>>();

    let response = builder
        .header("Content-Type", "application/vnd.amazon.eventstream")
        .body(Body::from(combined_body))
        .map_err(ServerError::ResponseBuild)?;

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_smithy_eventstream::frame::read_message_from;

    // Helper function to decode EventStream messages for testing
    fn decode_eventstream_message(
        data: &[u8],
    ) -> Result<(String, Bytes), Box<dyn std::error::Error>> {
        let message = read_message_from(data)?;

        let event_type = message
            .headers()
            .iter()
            .find(|h| h.name().as_str() == ":event-type")
            .and_then(|h| {
                if let aws_smithy_types::event_stream::HeaderValue::String(s) = h.value() {
                    Some(s.as_str().to_string())
                } else {
                    None
                }
            })
            .ok_or("Missing :event-type header")?;

        Ok((event_type, message.payload().clone()))
    }

    #[test]
    fn test_encode_payload_chunk() {
        let test_data = Bytes::from("Hello, streaming world!");

        let encoded =
            encode_payload_chunk(test_data.clone()).expect("Failed to encode payload chunk");

        // Verify the encoded message can be decoded
        let (event_type, payload) =
            decode_eventstream_message(&encoded).expect("Failed to decode EventStream message");

        assert_eq!(event_type, "PayloadChunk");
        assert_eq!(payload, test_data);
    }

    #[test]
    fn test_encode_invoke_complete_success() {
        let encoded = encode_invoke_complete(None, None).expect("Failed to encode InvokeComplete");

        // Verify the encoded message can be decoded
        let (event_type, payload) =
            decode_eventstream_message(&encoded).expect("Failed to decode EventStream message");

        assert_eq!(event_type, "InvokeComplete");

        // Parse the JSON payload
        let payload_json: serde_json::Value =
            serde_json::from_slice(&payload).expect("Failed to parse InvokeComplete payload");

        // Verify no error fields are present (or they are null)
        assert!(payload_json.get("ErrorCode").is_none() || payload_json["ErrorCode"].is_null());
        assert!(
            payload_json.get("ErrorDetails").is_none() || payload_json["ErrorDetails"].is_null()
        );
    }

    #[test]
    fn test_encode_invoke_complete_with_error() {
        let error_code = Some("RuntimeError".to_string());
        let error_details = Some("Function execution failed".to_string());

        let encoded = encode_invoke_complete(error_code.clone(), error_details.clone())
            .expect("Failed to encode InvokeComplete with error");

        // Verify the encoded message can be decoded
        let (event_type, payload) =
            decode_eventstream_message(&encoded).expect("Failed to decode EventStream message");

        assert_eq!(event_type, "InvokeComplete");

        // Parse the JSON payload
        let payload_json: serde_json::Value =
            serde_json::from_slice(&payload).expect("Failed to parse InvokeComplete payload");

        // Verify error fields are present
        assert_eq!(payload_json["ErrorCode"].as_str(), error_code.as_deref());
        assert_eq!(
            payload_json["ErrorDetails"].as_str(),
            error_details.as_deref()
        );
    }

    #[test]
    fn test_eventstream_message_structure() {
        // Test that the encoded messages have the correct EventStream structure
        let test_data = Bytes::from("test data");
        let encoded = encode_payload_chunk(test_data).expect("Failed to encode payload chunk");

        // EventStream messages should have a specific binary format
        // The first 12 bytes are the prelude (total length, headers length, prelude CRC)
        assert!(
            encoded.len() >= 16,
            "Message too short to be valid EventStream"
        );

        // Read the message length from the first 4 bytes (big-endian)
        let total_length = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);

        // The encoded length should match the total message length
        assert_eq!(
            total_length as usize,
            encoded.len(),
            "Message length mismatch"
        );
    }

    #[test]
    fn test_multiple_payload_chunks() {
        // Test encoding multiple chunks as would happen in a real stream
        let chunks = vec![
            Bytes::from("chunk 1"),
            Bytes::from("chunk 2"),
            Bytes::from("chunk 3"),
        ];

        let mut encoded_messages = Vec::new();

        for chunk in &chunks {
            let encoded =
                encode_payload_chunk(chunk.clone()).expect("Failed to encode payload chunk");
            encoded_messages.push(encoded);
        }

        // Add InvokeComplete at the end
        let invoke_complete =
            encode_invoke_complete(None, None).expect("Failed to encode InvokeComplete");
        encoded_messages.push(invoke_complete);

        // Verify we have the right number of messages
        assert_eq!(encoded_messages.len(), 4); // 3 chunks + 1 InvokeComplete

        // Verify each chunk can be decoded
        for (i, encoded) in encoded_messages.iter().enumerate() {
            let (event_type, _payload) =
                decode_eventstream_message(encoded).expect("Failed to decode EventStream message");

            if i < chunks.len() {
                assert_eq!(event_type, "PayloadChunk");
            } else {
                assert_eq!(event_type, "InvokeComplete");
            }
        }
    }
}
