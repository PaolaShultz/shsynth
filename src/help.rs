use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const HELP_MARKDOWN: &str = include_str!("../docs/HELP.md");
const WEB_HELP_PORT: u16 = 80;
const WEB_HELP_PATH: &str = "/help";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpKind {
    Blank,
    Heading,
    Link,
    Text,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelpLine {
    pub text: String,
    pub kind: HelpKind,
    pub anchor: Option<String>,
    pub target: Option<String>,
}

pub fn lines(width: usize) -> Vec<HelpLine> {
    let width = width.max(8);
    let mut out = Vec::new();
    for raw in HELP_MARKDOWN.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            out.push(HelpLine {
                text: String::new(),
                kind: HelpKind::Blank,
                anchor: None,
                target: None,
            });
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            let heading = heading.trim_start_matches('#').trim();
            out.push(HelpLine {
                text: truncate(heading, width),
                kind: HelpKind::Heading,
                anchor: Some(anchor(heading)),
                target: None,
            });
            continue;
        }
        let (text, target, link_only) = display_link(trimmed);
        let prefix = if link_only {
            "  "
        } else if text.starts_with("- ") {
            "- "
        } else {
            ""
        };
        let body = text.strip_prefix("- ").unwrap_or(&text);
        let kind = if target.is_some() {
            HelpKind::Link
        } else {
            HelpKind::Text
        };
        for (index, wrapped) in wrap(body, prefix, width).into_iter().enumerate() {
            out.push(HelpLine {
                text: wrapped,
                kind,
                anchor: None,
                target: (index == 0).then(|| target.clone()).flatten(),
            });
        }
    }
    while out.last().is_some_and(|line| line.kind == HelpKind::Blank) {
        out.pop();
    }
    out
}

pub fn target_index(lines: &[HelpLine], target: &str) -> Option<usize> {
    lines
        .iter()
        .position(|line| line.anchor.as_deref() == Some(target))
}

pub struct WebHelpServer {
    url: String,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl WebHelpServer {
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for WebHelpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebHelpUnavailable {
    NoLanIp,
    PortUnavailable,
}

impl WebHelpUnavailable {
    pub fn label(&self) -> &'static str {
        "web help unavailable"
    }
}

pub fn start_web_help() -> Result<WebHelpServer, WebHelpUnavailable> {
    let ip = lan_ipv4().ok_or(WebHelpUnavailable::NoLanIp)?;
    let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], WEB_HELP_PORT)))
        .map_err(|_| WebHelpUnavailable::PortUnavailable)?;
    listener
        .set_nonblocking(true)
        .map_err(|_| WebHelpUnavailable::PortUnavailable)?;
    let url = format!("http://{ip}{WEB_HELP_PATH}");
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let html = Arc::<str>::from(markdown_html());
    let handle = thread::spawn(move || serve(listener, thread_stop, html));
    Ok(WebHelpServer {
        url,
        stop,
        handle: Some(handle),
    })
}

fn display_link(raw: &str) -> (String, Option<String>, bool) {
    let Some(open) = raw.find('[') else {
        return (raw.to_owned(), None, false);
    };
    let Some(close) = raw[open + 1..].find(']').map(|index| open + 1 + index) else {
        return (raw.to_owned(), None, false);
    };
    let rest = &raw[close + 1..];
    let Some(rest) = rest.strip_prefix('(') else {
        return (raw.to_owned(), None, false);
    };
    let Some(end) = rest.find(')') else {
        return (raw.to_owned(), None, false);
    };
    let target = rest[..end].strip_prefix('#').map(str::to_owned);
    let label = &raw[open + 1..close];
    let suffix = &rest[end + 1..];
    let mut text = String::new();
    text.push_str(&raw[..open]);
    text.push_str(label);
    if target.is_some() {
        text.push_str(" >");
    }
    text.push_str(suffix);
    let link_only = raw[..open].trim().is_empty() && suffix.trim().is_empty();
    (text, target, link_only)
}

fn lan_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if usable_lan_ipv4(ip) => Some(ip),
        _ => None,
    }
}

fn usable_lan_ipv4(ip: Ipv4Addr) -> bool {
    !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_multicast()
        && !ip.is_broadcast()
        && !ip.is_documentation()
        && !ip.is_link_local()
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>, html: Arc<str>) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = handle_request(stream, &html);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
}

fn handle_request(mut stream: TcpStream, html: &str) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    stream.set_write_timeout(Some(Duration::from_millis(200)))?;
    let mut request = [0_u8; 1024];
    let read = stream.read(&mut request)?;
    let first_line = std::str::from_utf8(&request[..read])
        .ok()
        .and_then(|request| request.lines().next())
        .unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let (status, content_type, body) = if method == "GET" && path == WEB_HELP_PATH {
        ("200 OK", "text/html; charset=utf-8", html)
    } else {
        ("404 Not Found", "text/plain; charset=utf-8", "not found\n")
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n{body}",
        body.len()
    )
}

fn markdown_html() -> String {
    let mut html = String::from(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>SHR-DAW Help</title><style>\
         body{font-family:system-ui,sans-serif;line-height:1.5;margin:1.5rem auto;\
         max-width:44rem;padding:0 1rem;color:#111;background:#fdfdf9}\
         h1,h2,h3{line-height:1.2}a{color:#075f8f}p{margin:0 0 1rem}\
         .toc{margin:.2rem 0}.toc a{font-weight:600}\
         </style></head><body>",
    );
    let mut paragraph = Vec::new();
    for raw in HELP_MARKDOWN.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            flush_paragraph(&mut html, &mut paragraph);
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            flush_paragraph(&mut html, &mut paragraph);
            let level = trimmed.chars().take_while(|c| *c == '#').count().min(3);
            let heading = heading.trim_start_matches('#').trim();
            html.push_str(&format!(
                "<h{level} id=\"{}\">{}</h{level}>",
                anchor(heading),
                escape_html(heading)
            ));
            continue;
        }
        let (_, _, link_only) = display_link(trimmed);
        if link_only {
            flush_paragraph(&mut html, &mut paragraph);
            html.push_str("<p class=\"toc\">");
            html.push_str(&render_inline(trimmed));
            html.push_str("</p>");
        } else {
            paragraph.push(trimmed.to_owned());
        }
    }
    flush_paragraph(&mut html, &mut paragraph);
    html.push_str("</body></html>");
    html
}

fn flush_paragraph(html: &mut String, paragraph: &mut Vec<String>) {
    if paragraph.is_empty() {
        return;
    }
    html.push_str("<p>");
    html.push_str(&render_inline(&paragraph.join(" ")));
    html.push_str("</p>");
    paragraph.clear();
}

fn render_inline(raw: &str) -> String {
    let mut rendered = String::new();
    let mut rest = raw;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open + 1..].find(']').map(|index| open + 1 + index) else {
            break;
        };
        let after_label = &rest[close + 1..];
        let Some(after_open) = after_label.strip_prefix('(') else {
            break;
        };
        let Some(end) = after_open.find(')') else {
            break;
        };
        rendered.push_str(&escape_html(&rest[..open]));
        let label = &rest[open + 1..close];
        let target = &after_open[..end];
        if let Some(anchor) = target.strip_prefix('#') {
            rendered.push_str(&format!(
                "<a href=\"#{}\">{}</a>",
                escape_attr(anchor),
                escape_html(label)
            ));
        } else {
            rendered.push_str(&escape_html(label));
        }
        rest = &after_open[end + 1..];
    }
    rendered.push_str(&escape_html(rest));
    rendered
}

fn escape_html(raw: &str) -> String {
    let mut escaped = String::new();
    for character in raw.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn escape_attr(raw: &str) -> String {
    escape_html(raw)
}

fn wrap(text: &str, prefix: &str, width: usize) -> Vec<String> {
    let prefix_width = prefix.chars().count();
    let mut lines = Vec::new();
    let mut current = String::new();
    current.push_str(prefix);
    for word in text.split_whitespace() {
        let current_width = current.chars().count();
        let word_width = word.chars().count();
        let separator = usize::from(current_width > prefix_width);
        if current_width + separator + word_width > width && current_width > prefix_width {
            lines.push(current);
            current = " ".repeat(prefix_width);
        }
        if current.chars().count() > prefix_width {
            current.push(' ');
        }
        current.push_str(word);
    }
    if current.trim().is_empty() {
        lines.push(String::new());
    } else {
        lines.push(truncate(&current, width));
    }
    lines
}

fn anchor(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            out.push(character);
            dash = false;
        } else if (character.is_ascii_whitespace() || character == '-') && !out.is_empty() && !dash
        {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    text.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_links_target_headings() {
        let lines = lines(38);
        let links = lines
            .iter()
            .filter_map(|line| line.target.as_deref())
            .collect::<Vec<_>>();
        assert!(!links.is_empty());
        for target in links {
            assert!(target_index(&lines, target).is_some(), "missing {target}");
        }
    }

    #[test]
    fn help_lines_fit_forty_column_inner_width() {
        for line in lines(38) {
            assert!(line.text.chars().count() <= 38, "{line:?}");
        }
    }

    #[test]
    fn web_help_html_contains_internal_links_and_headings() {
        let html = markdown_html();
        assert!(html.contains("<h1 id=\"shr-daw-help\">SHR-DAW Help</h1>"));
        assert!(html.contains("<a href=\"#controller-basics\">Controller basics</a>"));
        assert!(html.contains("<h2 id=\"controller-basics\">Controller basics</h2>"));
    }

    #[test]
    fn request_handler_serves_only_help_path() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_request(stream, "<html>help</html>").unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(b"GET /help HTTP/1.1\r\nHost: test\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        handle.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.ends_with("<html>help</html>"));

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_request(stream, "<html>help</html>").unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .write_all(b"GET /other HTTP/1.1\r\nHost: test\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        handle.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 404 Not Found"));
        assert!(!response.contains("<html>help</html>"));
    }
}
