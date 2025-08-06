//! A custom Server-Sent Events (SSE) Event builder.
//!
//! This implementation provides fine-grained control over the SSE message format,
//! specifically ensuring a space is added after the `data:` field name, which
//! might be required by some non-compliant clients.

use std::fmt;

/// Represents a Server-Sent Event (SSE).
///
/// Use the builder pattern to construct an event, then use `to_string()`
/// or a `format!` macro to get the final string representation.
///
/// # Example
///
/// ```
/// # use chatspeed::ccproxy::helper::sse::Event;
/// let event = Event::default()
///     .id("1")
///     .event("message")
///     .data("Hello, world!")
///     .to_string();
///
/// assert_eq!(event, "id: 1\nevent: message\ndata: Hello, world!\n\n");
/// ```
#[derive(Debug, Default, Clone)]
pub struct Event {
    nl: Option<String>,
    event: Option<String>,
    data: Option<String>,
    text: Option<String>,
}

impl Event {
    pub fn set_gemini(mut self) -> Self {
        self.nl = Some("\r".to_string());
        self
    }

    /// Sets the `event` field (the event type).
    pub fn event<T: Into<String>>(mut self, event: T) -> Self {
        self.event = Some(event.into());
        self
    }

    /// Sets the `data` field from a string slice.
    /// Newlines in the data will be handled correctly by splitting them
    /// into multiple `data:` lines.
    pub fn data<T: Into<String>>(mut self, data: T) -> Self {
        self.data = Some(data.into());
        self
    }

    /// Sets the `text` field from a string slice.
    /// It'll print the text as is, without any prefix.
    pub fn text<T: Into<String>>(mut self, text: T) -> Self {
        self.text = Some(text.into());
        self
    }
}

/// Formats the Event into the correct SSE message format.
///
/// This implementation ensures a space is present after the `data:` prefix
/// and terminates the event with two newline characters.
impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nl = self.nl.as_deref().unwrap_or("\n");

        if let Some(event) = &self.event {
            write!(f, "event: {}{}", event, nl)?;
        }
        if let Some(data) = &self.data {
            if data.is_empty() {
                // 根据 SSE 规范，即使数据为空，也应发送 'data:' 行
                write!(f, "data: {}", nl)?;
            } else {
                for line in data.lines() {
                    // 每一行都加上 "data: "
                    write!(f, "data: {}{}", line, nl)?;
                }
            }
        }
        if let Some(text) = &self.text {
            if !text.is_empty() {
                write!(f, "{}{}", text, nl)?;
            }
        }

        // An event is terminated by an extra newline. `writeln!` adds one,
        // so we add the final one here to create the required blank line.
        f.write_str("\n")
    }
}

// --- Unit Tests ---
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_data_event_has_space() {
        let event = Event::default().data("test").to_string();
        assert_eq!(event, "data: test\n\n");
    }

    #[test]
    fn test_full_event_construction() {
        let event = Event::default()
            .event("update")
            .data("some data")
            .to_string();
        let expected = "event: update\ndata: some data\n\n";
        assert_eq!(event, expected);
    }

    #[test]
    fn test_multi_line_data() {
        let event = Event::default().data("line 1\nline 2").to_string();
        let expected = "data: line 1\ndata: line 2\n\n";
        assert_eq!(event, expected);
    }

    #[test]
    fn test_event_without_data() {
        let event = Event::default().event("ping").to_string();
        assert_eq!(event, "event: ping\n\n");
    }

    #[test]
    fn test_completely_empty_event() {
        // An event with no fields should just be the final terminator.
        let event = Event::default().to_string();
        assert_eq!(event, "\n");
    }

    #[test]
    fn test_empty_data_field_sends_data_line() {
        // According to SSE spec, even empty data sends a 'data:' line.
        let event = Event::default().data("").to_string();
        let expected = "data: \n\n"; // Now correctly sends "data: " followed by newline.
        assert_eq!(event, expected);
    }

    #[test]
    fn test_single_space_data_field() {
        // Sending a data field that contains only a space
        let event = Event::default().data(" ").to_string();
        let expected = "data:  \n\n"; // Expected "data: " + " " (from input) + newline
        assert_eq!(event, expected);
    }
}
