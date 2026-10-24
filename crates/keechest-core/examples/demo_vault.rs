//! Writes a vault with sample entries, for trying the app out:
//!
//! ```sh
//! cargo run -p keechest-core --example demo_vault -- demo.kchest
//! ```
//!
//! The master password is `demo-password`.

use keechest_core::{Entry, KdfParams, Vault, VaultFile};

fn main() -> keechest_core::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "demo.kchest".into());
    let mut vault = Vault::new("Демо");
    for (title, user, url, group, fav) in [
        (
            "Почта",
            "george@example.com",
            "https://mail.example.com",
            "Личное",
            true,
        ),
        (
            "GitHub",
            "georgebigscar-spec",
            "https://github.com",
            "Работа",
            true,
        ),
        (
            "Банк",
            "4276 **** 1234",
            "https://bank.example.com",
            "Финансы",
            false,
        ),
        ("Wi-Fi дома", "", "", "Личное", false),
        (
            "Jira",
            "g.bigscar",
            "https://jira.example.com",
            "Работа",
            false,
        ),
    ] {
        let mut e = Entry::new();
        e.title = title.into();
        e.username = user.into();
        e.url = url.into();
        e.group = group.into();
        e.favorite = fav;
        e.password = keechest_core::GeneratorOptions::default()
            .generate()
            .unwrap()
            .to_string();
        if title == "Банк" {
            e.fields.push(keechest_core::CustomField::new(
                "Кодовое слово",
                "сирень",
                true,
            ));
            e.fields.push(keechest_core::CustomField::new(
                "Отделение",
                "ул. Арбат, 1",
                false,
            ));
        }
        if title == "Wi-Fi дома" {
            e.notes = "Сеть: HomeNet_5G\nРоутер в коридоре.".into();
        }
        vault.upsert(e);
    }
    let kdf = KdfParams {
        memory_kib: 64 * 1024,
        iterations: 3,
        parallelism: 4,
    };
    VaultFile::create(&path, "demo-password", kdf, &vault)?;
    println!("wrote {path}, master password: demo-password");
    Ok(())
}
