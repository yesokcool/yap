//! Links to places and channels (PROTOCOL.md › Links to places and
//! channels): `<server>/#place=space` and `<server>/#channel=<id>`
//! (`&message=<id>` jumps to a message). No invite key and no Personal
//! space: a link is for people who already use Yap.

use crate::config::{percent_decode, percent_encode};

/// Where a link to here points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `#place=…`: any name (whether this server has that place is for the caller to say).
    Place(String),
    /// `#channel=<id>[&message=<id>]`.
    Channel { id: String, message: Option<String> },
}

/// The link to a place on `server`: `https://chat.example.com/#place=space`.
pub fn place_link(server: &str, place: &str) -> String {
    format!("{}/#place={}", server.trim_end_matches('/'), percent_encode(place))
}

/// The link to a text channel on `server`, its id encoded like the web's `encodeURIComponent`.
pub fn channel_link(server: &str, id: &str) -> String {
    format!("{}/#channel={}", server.trim_end_matches('/'), percent_encode(id))
}

/// `url` as a link to a place or channel on `server`, or None: another
/// server's (or another page's, or one with a query) link, or an invite or
/// sign-in link (`key=`, `handoff=`), which stay as they are.
pub fn parse(server: &str, url: &str) -> Option<Target> {
    let (base, fragment) = url.trim().split_once('#')?;
    let (origin, path) = origin_path(base)?;
    let (ours, root) = origin_path(server.trim_end_matches('/'))?;
    if origin != ours || !(path == root || path.strip_suffix('/') == Some(root)) {
        return None;
    }
    // Like the web's URLSearchParams: the first of each, `+` is a space.
    let (mut place, mut channel, mut message) = (None, None, None);
    for kv in fragment.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        let slot = match percent_decode(k).as_str() {
            "key" | "handoff" => return None,
            "place" => &mut place,
            "channel" => &mut channel,
            "message" => &mut message,
            _ => continue,
        };
        slot.get_or_insert_with(|| percent_decode(v));
    }
    let given = |s: Option<String>| s.filter(|s| !s.is_empty());
    match (given(place), given(channel)) {
        (Some(p), _) => Some(Target::Place(p)),
        (None, Some(id)) => Some(Target::Channel { id, message: given(message) }),
        (None, None) => None,
    }
}

/// An http(s) URL's origin (lowercase, without its default port) and the
/// rest up to the fragment (path and query); None for any other scheme.
fn origin_path(url: &str) -> Option<(String, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let port = match scheme.as_str() {
        "https" => ":443",
        "http" => ":80",
        _ => return None,
    };
    let (host, path) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let host = host.to_ascii_lowercase();
    let host = host.strip_suffix(port).unwrap_or(&host);
    Some((format!("{scheme}://{host}"), path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: &str = "https://chat.example.com";

    fn place(p: &str) -> Option<Target> {
        Some(Target::Place(p.into()))
    }

    fn channel(id: &str, message: Option<&str>) -> Option<Target> {
        Some(Target::Channel { id: id.into(), message: message.map(Into::into) })
    }

    #[test]
    fn builds_links() {
        assert_eq!(place_link(SERVER, "space"), "https://chat.example.com/#place=space");
        assert_eq!(place_link("http://localhost:8787/", "when"), "http://localhost:8787/#place=when");
        assert_eq!(channel_link(SERVER, "general"), "https://chat.example.com/#channel=general");
        // An id that isn't plain letters goes encoded, and reads back the same.
        let odd = "movie night/#2&more=yes";
        let link = channel_link(SERVER, odd);
        assert_eq!(link, "https://chat.example.com/#channel=movie%20night%2F%232%26more%3Dyes");
        assert_eq!(parse(SERVER, &link), channel(odd, None));
        for p in ["board", "draw", "space", "when", "watch"] {
            assert_eq!(parse(SERVER, &place_link(SERVER, p)), place(p), "{p}");
        }
    }

    #[test]
    fn own_server_only() {
        assert_eq!(parse(SERVER, "https://chat.example.com/#place=space"), place("space"));
        assert_eq!(parse(SERVER, "https://chat.example.com#place=draw"), place("draw"));
        // Case, the default port and a trailing slash on the server don't matter.
        assert_eq!(parse(SERVER, "HTTPS://Chat.Example.com:443/#place=watch"), place("watch"));
        assert_eq!(parse("https://chat.example.com/", "https://chat.example.com/#place=when"), place("when"));
        assert_eq!(parse("http://localhost:8787", "http://localhost:8787/#channel=c1"), channel("c1", None));
        // Another server, scheme, port or page: the browser's.
        assert_eq!(parse(SERVER, "https://other.example.com/#place=space"), None);
        assert_eq!(parse(SERVER, "https://chat.example.com.evil.com/#place=space"), None);
        assert_eq!(parse(SERVER, "https://evil.com@chat.example.com/#place=space"), None);
        assert_eq!(parse(SERVER, "http://chat.example.com/#place=space"), None);
        assert_eq!(parse(SERVER, "https://chat.example.com:8443/#place=space"), None);
        assert_eq!(parse(SERVER, "https://chat.example.com/board#place=space"), None);
        assert_eq!(parse(SERVER, "https://chat.example.com/?x=1#place=space"), None);
        assert_eq!(parse(SERVER, "https://chat.example.com/privacy"), None);
        assert_eq!(parse(SERVER, "https://chat.example.com/"), None);
        assert_eq!(parse(SERVER, "mailto:a@chat.example.com#place=space"), None);
        // A server under a path has its links there.
        assert_eq!(parse("https://example.com/yap", "https://example.com/yap/#place=space"), place("space"));
        assert_eq!(parse("https://example.com/yap", "https://example.com/#place=space"), None);
    }

    #[test]
    fn invites_and_sign_in_links_stay_as_they_are() {
        assert_eq!(parse(SERVER, "https://chat.example.com/#key=K"), None);
        assert_eq!(parse(SERVER, "https://chat.example.com/#key=K&place=space"), None);
        assert_eq!(parse(SERVER, "https://chat.example.com/#place=watch&handoff=h&name=Ada"), None);
        assert_eq!(parse(SERVER, "https://chat.example.com/#channel=c1&key"), None);
        assert_eq!(parse(SERVER, "https://chat.example.com/#embed=1"), None);
    }

    #[test]
    fn reads_the_fragment_like_the_web() {
        // Any place name comes back (the app says whether it has it).
        assert_eq!(parse(SERVER, "https://chat.example.com/#place=moon"), place("moon"));
        assert_eq!(parse(SERVER, "https://chat.example.com/#channel=c1&message=m9"), channel("c1", Some("m9")));
        assert_eq!(parse(SERVER, "https://chat.example.com/#message=m9&channel=c1&embed=1"), channel("c1", Some("m9")));
        assert_eq!(parse(SERVER, "https://chat.example.com/#channel=c1&message="), channel("c1", None));
        // A place wins over a channel; the first of each counts; an empty one is none.
        assert_eq!(parse(SERVER, "https://chat.example.com/#channel=c1&place=space"), place("space"));
        assert_eq!(parse(SERVER, "https://chat.example.com/#place=draw&place=space"), place("draw"));
        assert_eq!(parse(SERVER, "https://chat.example.com/#place=&channel=c1"), channel("c1", None));
        assert_eq!(parse(SERVER, "https://chat.example.com/#channel="), None);
        // Encoded ids, `+` as a space (URLSearchParams), a bad escape kept as it is.
        assert_eq!(parse(SERVER, "https://chat.example.com/#channel=caf%C3%A9%2Fnight"), channel("café/night", None));
        assert_eq!(parse(SERVER, "https://chat.example.com/#channel=movie+night"), channel("movie night", None));
        assert_eq!(parse(SERVER, "https://chat.example.com/#channel=50%zz"), channel("50%zz", None));
        assert_eq!(parse(SERVER, "https://chat.example.com/#%63hannel=c1"), channel("c1", None));
    }
}
