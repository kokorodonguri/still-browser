//! Saved logins imported from another browser's password export. The shell
//! keeps them in `passwords.dat`, encrypted for the Windows user (DPAPI);
//! this module never touches the disk. Passwords never reach the interface:
//! it sees only the site and user name, and a password is placed into a page
//! only when the user picks the login for that page.

use serde::{Deserialize, Serialize};
use url::Url;

/// Saved logins beyond this are not imported.
pub const LOGIN_LIMIT: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Login {
    /// Origin of the sign-in page: `https://host[:port]`.
    pub url: String,
    pub username: String,
    pub password: String,
}

impl Login {
    /// `None` unless `url` is an http(s) address with a host.
    pub fn new(url: &str, username: &str, password: &str) -> Option<Self> {
        let parsed = Url::parse(url.trim()).ok().filter(|u| matches!(u.scheme(), "http" | "https"))?;
        parsed.host_str()?;
        Some(Self { url: parsed.origin().ascii_serialization(), username: username.trim().into(), password: password.into() })
    }

    pub fn host(&self) -> &str {
        self.url.split_once("://").map_or("", |(_, rest)| rest.split(':').next().unwrap_or(rest))
    }

    /// Whether this login belongs on `page`: the same host, or one host is
    /// a subdomain of the other (`accounts.example.com` and `example.com`).
    /// A login saved for https is never offered on an http page.
    pub fn matches(&self, page: &Url) -> bool {
        let Some(host) = page.host_str() else { return false };
        let secure = self.url.starts_with("https://");
        let scheme_ok = match page.scheme() {
            "https" => true,
            "http" => !secure,
            _ => false,
        };
        let saved = self.host();
        let related = |long: &str, short: &str| short.contains('.') && long.strip_suffix(short).is_some_and(|p| p.ends_with('.'));
        scheme_ok && (host == saved || related(host, saved) || related(saved, host))
    }
}

/// What the interface may know about a login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoginView {
    /// Position in the vault, used to fill or remove it.
    pub index: usize,
    pub host: String,
    pub username: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Vault {
    #[serde(default)]
    pub logins: Vec<Login>,
}

impl Vault {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("vault serializes")
    }

    /// Adds logins; one already saved for the same site and user name gets
    /// the imported password. Returns how many were added or changed.
    pub fn merge(&mut self, imported: Vec<Login>) -> usize {
        let mut count = 0;
        for login in imported {
            let full = self.logins.len() >= LOGIN_LIMIT;
            match self.logins.iter_mut().find(|l| l.url == login.url && l.username == login.username) {
                Some(existing) if existing.password != login.password => {
                    existing.password = login.password;
                    count += 1;
                }
                Some(_) => {}
                None if !full => {
                    self.logins.push(login);
                    count += 1;
                }
                None => {}
            }
        }
        if count > 0 {
            self.logins.sort_by(|a, b| (a.host(), &a.username).cmp(&(b.host(), &b.username)));
        }
        count
    }

    pub fn get(&self, index: usize) -> Option<&Login> {
        self.logins.get(index)
    }

    pub fn remove(&mut self, index: usize) -> bool {
        (index < self.logins.len()).then(|| self.logins.remove(index)).is_some()
    }

    pub fn clear(&mut self) -> bool {
        let had = !self.logins.is_empty();
        self.logins.clear();
        had
    }

    pub fn len(&self) -> usize {
        self.logins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.logins.is_empty()
    }

    fn view(index: usize, login: &Login) -> LoginView {
        LoginView { index, host: login.host().into(), username: login.username.clone() }
    }

    /// Logins for the page at `url`, exact host first.
    pub fn for_page(&self, url: &str) -> Vec<LoginView> {
        let Ok(page) = Url::parse(url) else { return Vec::new() };
        let mut found: Vec<LoginView> = self.logins.iter().enumerate().filter(|(_, l)| l.matches(&page)).map(|(i, l)| Self::view(i, l)).collect();
        found.sort_by_key(|v| Some(v.host.as_str()) != page.host_str());
        found
    }

    pub fn views(&self) -> Vec<LoginView> {
        self.logins.iter().enumerate().map(|(i, l)| Self::view(i, l)).collect()
    }
}

/// Script that fills a login into the page's sign-in form. It first checks
/// that the page is still on `host` (it may have navigated away since the
/// login was offered). Fields hidden from the user are never filled. Pages
/// that ask for the user name and password on separate steps get whichever
/// field is showing. Evaluates to whether anything was filled.
pub fn fill_script(host: &str, login: &Login) -> String {
    let args = serde_json::json!([host, login.username, login.password]);
    format!(
        r#"((host, user, pass) => {{
  if (location.hostname !== host) return false;
  const shown = (el) => el && !el.disabled && !el.readOnly && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
  const fields = [...document.querySelectorAll("input")].filter(shown);
  const password = fields.find((el) => el.type === "password");
  const named = (el) => /^(text|email|tel)$/.test(el.type) && /user|mail|login|account|id/i.test(`${{el.autocomplete}} ${{el.name}} ${{el.id}}`);
  let name = fields.find((el) => /username|email/.test(el.autocomplete));
  if (!name && password) {{
    const before = fields.slice(0, fields.indexOf(password)).filter((el) => /^(text|email|tel)$/.test(el.type));
    name = before[before.length - 1];
  }}
  if (!name && !password) name = fields.find(named) ?? fields.find((el) => el.type === "email");
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  const put = (el, value) => {{
    el.focus();
    setValue.call(el, value);
    el.dispatchEvent(new Event("input", {{ bubbles: true }}));
    el.dispatchEvent(new Event("change", {{ bubbles: true }}));
  }};
  if (name && user) put(name, user);
  if (password) put(password, pass);
  return !!((name && user) || password);
}})(...{args})"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(url: &str, user: &str, pass: &str) -> Login {
        Login::new(url, user, pass).unwrap()
    }

    #[test]
    fn logins_keep_only_the_origin() {
        assert_eq!(login("https://Accounts.Example.com/signin?x=1", " me ", "pw").url, "https://accounts.example.com");
        assert_eq!(login("http://a.test:8080/x", "me", "pw").url, "http://a.test:8080");
        assert_eq!(login("http://a.test:8080/x", "me", "pw").host(), "a.test");
        assert!(Login::new("android://hash@com.app/", "me", "pw").is_none());
        assert!(Login::new("not a url", "me", "pw").is_none());
    }

    #[test]
    fn matches_the_same_site_and_its_subdomains_only() {
        let saved = login("https://accounts.example.com", "me", "pw");
        let page = |u: &str| Url::parse(u).unwrap();
        assert!(saved.matches(&page("https://accounts.example.com/signin")));
        assert!(saved.matches(&page("https://example.com/login")), "parent domain");
        assert!(saved.matches(&page("https://eu.accounts.example.com/")), "subdomain");
        assert!(!saved.matches(&page("https://other.example.com/")), "sibling");
        assert!(!saved.matches(&page("https://notexample.com/")));
        assert!(!saved.matches(&page("https://example.com.evil.test/")));
        assert!(!saved.matches(&page("http://accounts.example.com/")), "https login on an http page");
        assert!(login("http://a.test", "me", "pw").matches(&page("https://a.test/")));
        assert!(!login("https://localhost", "me", "pw").matches(&page("https://evil.localhost/")), "a dotless host has no subdomains");
        assert!(!login("https://example.com", "me", "pw").matches(&page("https://com/")), "a bare suffix never matches");
    }

    #[test]
    fn merging_updates_passwords_and_skips_duplicates() {
        let mut vault = Vault::default();
        assert_eq!(vault.merge(vec![login("https://b.test", "me", "1"), login("https://a.test", "me", "1")]), 2);
        assert_eq!(vault.logins[0].url, "https://a.test", "sorted by site");
        assert_eq!(vault.merge(vec![login("https://a.test", "me", "1"), login("https://a.test", "me", "2"), login("https://a.test", "you", "3")]), 2);
        assert_eq!(vault.len(), 3);
        assert_eq!(vault.logins.iter().find(|l| l.username == "me" && l.host() == "a.test").unwrap().password, "2");
        let round = Vault::parse(&vault.to_json()).unwrap();
        assert_eq!(round, vault);
        assert!(vault.remove(0) && !vault.remove(10));
        assert!(vault.clear() && vault.is_empty() && !vault.clear());
    }

    #[test]
    fn page_logins_list_the_exact_host_first_without_passwords() {
        let mut vault = Vault::default();
        vault.merge(vec![login("https://example.com", "parent", "x"), login("https://www.example.com", "exact", "y"), login("https://other.test", "o", "z")]);
        let found = vault.for_page("https://www.example.com/login");
        assert_eq!(found.iter().map(|v| v.username.as_str()).collect::<Vec<_>>(), vec!["exact", "parent"]);
        assert_eq!(vault.get(found[0].index).unwrap().password, "y");
        assert!(!serde_json::to_string(&found).unwrap().contains("\"y\""));
        assert!(vault.for_page("about:blank").is_empty());
    }

    #[test]
    fn fill_script_quotes_its_values() {
        let script = fill_script("a.test", &login("https://a.test", "me\"</script>", "p'w`${x}"));
        assert!(script.contains(r#"["a.test","me\"</script>","p'w`${x}"]"#));
        assert!(script.contains("location.hostname !== host"));
    }
}
