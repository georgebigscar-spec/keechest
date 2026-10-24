//! Importing from KeePass and CSV exports of other password managers, and
//! exporting to CSV.

use std::collections::HashMap;

use keepass::db::{EntryRef, GroupRef};
use keepass::{Database, DatabaseKey};
use zeroize::Zeroizing;

use crate::{CustomField, Entry, Error, HistoryItem, Result, Vault};

/// Reads the entries of a KeePass database (KDBX 3 or 4, or old KDB),
/// skipping the recycle bin. Groups become `Parent / Child` group names;
/// extra string fields become custom fields, protected ones secret.
pub fn import_kdbx(file: &[u8], password: &str, keyfile: Option<&[u8]>) -> Result<Vec<Entry>> {
    let mut key = DatabaseKey::new();
    if !password.is_empty() {
        key = key.with_password(password);
    }
    if let Some(mut kf) = keyfile {
        key = key
            .with_keyfile(&mut kf)
            .map_err(|e| Error::Import(format!("key file: {e}")))?;
    }
    let db = Database::parse(file, key).map_err(|e| match e {
        // keepass doesn't export its key error type, only this variant.
        keepass::db::DatabaseOpenError::Key(k) if k.to_string() == "Incorrect key" => {
            Error::WrongPasswordOrCorrupted
        }
        e => Error::Import(e.to_string()),
    })?;
    let bin = db.recycle_bin().map(|g| g.id());
    let mut out = Vec::new();
    walk(&db.root(), "", bin, &mut out);
    Ok(out)
}

fn walk(group: &GroupRef<'_>, path: &str, bin: Option<keepass::db::GroupId>, out: &mut Vec<Entry>) {
    for e in group.entries() {
        out.push(convert(&e, path));
    }
    for child in group.groups() {
        if Some(child.id()) == bin {
            continue;
        }
        let name = child.name.trim();
        let child_path = match (path.is_empty(), name.is_empty()) {
            (_, true) => path.to_owned(),
            (true, false) => name.to_owned(),
            (false, false) => format!("{path} / {name}"),
        };
        walk(&child, &child_path, bin, out);
    }
}

fn timestamp(t: Option<chrono::NaiveDateTime>) -> Option<u64> {
    t.and_then(|t| u64::try_from(t.and_utc().timestamp()).ok())
}

const STANDARD_FIELDS: [&str; 5] = ["Title", "UserName", "Password", "URL", "Notes"];

fn convert(src: &EntryRef<'_>, group: &str) -> Entry {
    let get = |k: &str| src.get(k).unwrap_or_default().to_owned();
    let mut e = Entry::new();
    e.title = get("Title");
    e.username = get("UserName");
    e.password = get("Password");
    e.url = get("URL");
    e.notes = get("Notes");
    e.group = group.to_owned();
    let mut extra: Vec<(&String, &keepass::db::Value<String>)> = src
        .fields
        .iter()
        .filter(|(k, v)| !STANDARD_FIELDS.contains(&k.as_str()) && !v.get().is_empty())
        .collect();
    extra.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in extra {
        e.fields.push(CustomField::new(
            k.as_str(),
            v.get().as_str(),
            v.is_protected(),
        ));
    }
    if let Some(t) = timestamp(src.times.creation) {
        e.created = t;
    }
    if let Some(t) = timestamp(src.times.last_modification) {
        e.modified = t;
    }
    if let Some(history) = &src.history {
        // KeePass keeps whole old versions, oldest first; keep the distinct
        // older passwords, newest first.
        for old in history.get_entries().iter().rev() {
            let pw = old.get_password().unwrap_or_default();
            if pw.is_empty() || pw == e.password || e.history.iter().any(|h| h.password == pw) {
                continue;
            }
            let mut item = HistoryItem::default();
            item.password = pw.to_owned();
            item.replaced = timestamp(old.times.last_modification).unwrap_or(0);
            e.history.push(item);
        }
        e.history.truncate(crate::vault::HISTORY_LEN);
    }
    e
}

/// Column names (lowercased) other managers use for each field, in order of
/// preference. Covers KeePassXC, Bitwarden, Chrome, Firefox and 1Password
/// style exports.
const COLUMNS: [(Field, &[&str]); 7] = [
    (Field::Title, &["title", "name", "account", "название"]),
    (
        Field::Username,
        &[
            "username",
            "login_username",
            "user name",
            "login",
            "user",
            "email",
            "логин",
        ],
    ),
    (Field::Password, &["password", "login_password", "пароль"]),
    (
        Field::Url,
        &["url", "login_uri", "website", "uri", "web site", "адрес"],
    ),
    (
        Field::Notes,
        &["notes", "note", "comments", "extra", "заметки"],
    ),
    (Field::Group, &["group", "folder", "grouping", "группа"]),
    (Field::Favorite, &["favorite", "fav", "избранное"]),
];

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Field {
    Title,
    Username,
    Password,
    Url,
    Notes,
    Group,
    Favorite,
}

/// Reads a CSV export with a header row. Columns are matched by name; a
/// password column is required. Rows without a title are named after their
/// URL's host.
pub fn import_csv(data: &[u8]) -> Result<Vec<Entry>> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(data.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(data));
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| Error::Import(e.to_string()))?
        .iter()
        .map(|h| h.trim().to_lowercase())
        .collect();
    let mut index: HashMap<Field, usize> = HashMap::new();
    for (field, names) in COLUMNS {
        if let Some(i) = names
            .iter()
            .find_map(|n| headers.iter().position(|h| h == n))
        {
            index.insert(field, i);
        }
    }
    if !index.contains_key(&Field::Password) {
        return Err(Error::Import("no password column in the CSV header".into()));
    }
    let mut out = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| Error::Import(e.to_string()))?;
        let get = |f: Field| {
            index
                .get(&f)
                .and_then(|&i| record.get(i))
                .unwrap_or_default()
                .trim()
        };
        let mut e = Entry::new();
        e.title = get(Field::Title).to_owned();
        e.username = get(Field::Username).to_owned();
        e.password = record
            .get(index[&Field::Password])
            .unwrap_or_default()
            .to_owned();
        e.url = get(Field::Url).to_owned();
        e.notes = get(Field::Notes).to_owned();
        let group = get(Field::Group);
        // KeePassXC writes the root group's name first.
        e.group = group
            .strip_prefix("Root/")
            .or_else(|| (group == "Root").then_some(""))
            .unwrap_or(group)
            .replace('/', " / ");
        e.favorite = matches!(
            get(Field::Favorite).to_lowercase().as_str(),
            "1" | "true" | "yes" | "да"
        );
        if e.title.is_empty() {
            e.title = host(&e.url).unwrap_or_default().to_owned();
        }
        if e.title.is_empty() && e.username.is_empty() && e.password.is_empty() {
            continue;
        }
        out.push(e);
    }
    Ok(out)
}

/// The host part of a URL, without `www.`.
fn host(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?.split(':').next()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    (!host.is_empty()).then_some(host)
}

/// The whole vault as CSV with KeePassXC's columns plus `Favorite`.
/// Custom fields go to the end of the notes as `name: value` lines, since
/// other managers have no common column for them. The result is
/// plaintext; it is wiped when dropped, the file it is written to is not.
pub fn export_csv(vault: &Vault) -> Result<Zeroizing<Vec<u8>>> {
    let size: usize = vault
        .entries
        .iter()
        .map(|e| {
            e.title.len()
                + e.username.len()
                + e.password.len()
                + e.url.len()
                + e.notes.len()
                + e.group.len()
                + e.fields
                    .iter()
                    .map(|f| f.name.len() + f.value.len() + 3)
                    .sum::<usize>()
        })
        .sum();
    // Room for quoting, so the buffer is unlikely to be reallocated.
    let mut buf = Zeroizing::new(Vec::with_capacity(
        size * 2 + 64 * vault.entries.len() + 128,
    ));
    {
        let mut w = csv::Writer::from_writer(&mut *buf);
        let csv_err = |e: csv::Error| Error::Import(e.to_string());
        w.write_record([
            "Group", "Title", "Username", "Password", "URL", "Notes", "Favorite",
        ])
        .map_err(csv_err)?;
        for e in &vault.entries {
            let mut notes = Zeroizing::new(e.notes.clone());
            for f in &e.fields {
                if !notes.is_empty() {
                    notes.push('\n');
                }
                notes.push_str(&f.name);
                notes.push_str(": ");
                notes.push_str(&f.value);
            }
            w.write_record([
                e.group.as_str(),
                e.title.as_str(),
                e.username.as_str(),
                e.password.as_str(),
                e.url.as_str(),
                notes.as_str(),
                if e.favorite { "1" } else { "" },
            ])
            .map_err(csv_err)?;
        }
        w.flush()?;
    }
    Ok(buf)
}

/// Writes [`export_csv`] to `path`, readable only by the owner on Unix.
pub fn export_csv_file(vault: &Vault, path: &std::path::Path) -> Result<()> {
    use std::io::Write;
    let csv = export_csv(vault)?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(&csv)?;
    f.sync_all()?;
    Ok(())
}

impl Vault {
    /// Adds `entries`, skipping any with the same title, username, password
    /// and URL as one already in the vault. Returns (added, skipped).
    pub fn merge(&mut self, entries: Vec<Entry>) -> (usize, usize) {
        let same = |a: &Entry, b: &Entry| {
            a.title == b.title
                && a.username == b.username
                && a.password == b.password
                && a.url == b.url
        };
        let (mut added, mut skipped) = (0, 0);
        for e in entries {
            if self.entries.iter().any(|x| same(x, &e)) {
                skipped += 1;
            } else {
                self.entries.push(e);
                added += 1;
            }
        }
        (added, skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(host("https://www.github.com/login?x=1"), Some("github.com"));
        assert_eq!(host("http://user@example.org:8080"), Some("example.org"));
        assert_eq!(host("mail.ru"), Some("mail.ru"));
        assert_eq!(host(""), None);
    }

    #[test]
    fn keepassxc_csv() {
        let csv = "\"Group\",\"Title\",\"Username\",\"Password\",\"URL\",\"Notes\",\"TOTP\"\n\
                   \"Root/Work\",\"GitHub\",\"me\",\"p,a\"\"ss\",\"https://github.com\",\"line1\nline2\",\"\"\n\
                   \"Root\",\"Bank\",\"\",\"x\",\"\",\"\",\"\"\n";
        let got = import_csv(csv.as_bytes()).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].group, "Work");
        assert_eq!(got[0].password, "p,a\"ss");
        assert_eq!(got[0].notes, "line1\nline2");
        assert_eq!(got[1].group, "");
    }

    #[test]
    fn bitwarden_and_firefox_csv() {
        let bw = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
                  Social,1,login,Twitter,,,0,https://twitter.com,bird,tweet,\n";
        let got = import_csv(bw.as_bytes()).unwrap();
        assert_eq!(got[0].title, "Twitter");
        assert_eq!(got[0].username, "bird");
        assert_eq!(got[0].password, "tweet");
        assert_eq!(got[0].group, "Social");
        assert!(got[0].favorite);

        let ff = "\u{feff}\"url\",\"username\",\"password\",\"httpRealm\"\n\"https://www.example.com\",\"a\",\"b\",\"\"\n";
        let got = import_csv(ff.as_bytes()).unwrap();
        assert_eq!(got[0].title, "example.com");
    }

    #[test]
    fn csv_without_password_column_fails() {
        assert!(import_csv(b"name,url\na,b\n").is_err());
    }

    #[test]
    fn export_roundtrips_through_import() {
        let mut v = Vault::new("t");
        let mut e = Entry::new();
        e.title = "Почта".into();
        e.username = "me".into();
        e.password = "a\"b,c\nd".into();
        e.group = "Личное".into();
        e.favorite = true;
        e.fields.push(CustomField::new("PIN", "0000", true));
        v.upsert(e);
        let csv = export_csv(&v).unwrap();
        let back = import_csv(&csv).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].title, "Почта");
        assert_eq!(back[0].password, "a\"b,c\nd");
        assert_eq!(back[0].group, "Личное");
        assert_eq!(back[0].notes, "PIN: 0000");
        assert!(back[0].favorite);
    }

    #[test]
    fn merge_skips_duplicates() {
        let mut v = Vault::new("t");
        let mut e = Entry::new();
        e.title = "a".into();
        e.password = "p".into();
        v.upsert(e.clone());
        let mut other = Entry::new();
        other.title = "b".into();
        assert_eq!(v.merge(vec![e, other]), (1, 1));
        assert_eq!(v.entries.len(), 2);
    }

    fn sample_kdbx(password: &str) -> Vec<u8> {
        let mut db = Database::new();
        {
            let mut root = db.root_mut();
            let mut e = root.add_entry();
            e.set_unprotected("Title", "Top");
            e.set_protected("Password", "top-secret");
            let mut work = root.add_group();
            work.name = "Work".into();
            let mut sub = work.add_group();
            sub.name = "Servers".into();
            let mut e = sub.add_entry();
            e.set_unprotected("Title", "SSH");
            e.set_unprotected("UserName", "root");
            e.set_protected("Password", "hunter2");
            e.set_unprotected("URL", "ssh://example.com");
            e.set_unprotected("Notes", "rack 4");
            e.set_protected("PIN", "1234");
        }
        let mut out = Vec::new();
        db.save(&mut out, DatabaseKey::new().with_password(password))
            .unwrap();
        out
    }

    #[test]
    fn kdbx_import() {
        let file = sample_kdbx("kp");
        let mut got = import_kdbx(&file, "kp", None).unwrap();
        got.sort_by(|a, b| a.title.cmp(&b.title));
        assert_eq!(got.len(), 2);
        let ssh = &got[0];
        assert_eq!(ssh.title, "SSH");
        assert_eq!(ssh.username, "root");
        assert_eq!(ssh.password, "hunter2");
        assert_eq!(ssh.group, "Work / Servers");
        assert_eq!(ssh.notes, "rack 4");
        assert!(ssh.fields == [CustomField::new("PIN", "1234", true)]);
        assert_eq!(got[1].group, "");
        assert_eq!(got[1].password, "top-secret");

        assert!(matches!(
            import_kdbx(&file, "wrong", None),
            Err(Error::WrongPasswordOrCorrupted)
        ));
        assert!(import_kdbx(b"not a database", "kp", None).is_err());
    }
}
