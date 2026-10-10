use hoardini_core::apps::{
    self, AppInputs, RarelyUsedApp, SteamApp, Uninstaller, parse_app_manifest,
    parse_library_folders, split_command,
};
use hoardini_core::index::{FakeFs, RefreshOptions, refresh};
use hoardini_core::store::Db;
use hoardini_win::{InstalledProgram, UserAssistEntry};

const GIB: u64 = 1 << 30;
const DAY: i64 = 86_400_000;
const NOW: i64 = 1_800_000_000_000;

pub fn program(name: &str, loc: Option<&str>, uninstall: Option<&str>) -> InstalledProgram {
    InstalledProgram {
        name: name.into(),
        install_location: loc.map(str::to_string),
        uninstall_key: format!(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{name}"),
        uninstall_string: uninstall.map(str::to_string),
        ..Default::default()
    }
}

pub fn ran(exe: &str, days_ago: i64) -> UserAssistEntry {
    UserAssistEntry {
        name: exe.into(),
        run_count: 3,
        last_run_ms: Some(NOW - days_ago * DAY),
    }
}

pub fn steam_game(appid: u32, name: &str, played_days_ago: Option<i64>) -> SteamApp {
    SteamApp {
        appid,
        name: name.into(),
        dir: format!(r"D:\Steam\steamapps\common\{name}"),
        last_played_ms: played_days_ago.map(|d| NOW - d * DAY),
    }
}

pub fn fake_disk(db: &Db) {
    let fs = FakeFs::new(r"D:\");
    for (path, size) in [
        (r"D:\Apps\Big\big.dat", 3 * GIB),
        (r"D:\Apps\Fresh\f.dat", 3 * GIB),
        (r"D:\Apps\Never\n.dat", 4 * GIB),
        (r"D:\Apps\Tiny\t.dat", GIB),
        (r"D:\Apps\Hidden\h.dat", 3 * GIB),
        (r"D:\Apps\NoUninstall\x.dat", 3 * GIB),
        (r"D:\Program Files\Wide\w.dat", 3 * GIB),
        (r"D:\Steam\steam.dat", GIB),
        (r"D:\Steam\steamapps\common\Old Game\game.pak", 5 * GIB),
        (r"D:\Steam\steamapps\common\New Game\game.pak", 6 * GIB),
        (r"D:\Steam\steamapps\common\Unplayed\u.pak", 5 * GIB / 2),
        (r"D:\Apps\Code\Code.exe", 100 * (1 << 20)),
        (r"D:\Apps\Code\resources.dat", 3 * GIB),
        (r"D:\Apps\Office\root\WINWORD.EXE", 2 * (1 << 20)),
        (r"D:\Apps\Office\data.dat", 3 * GIB),
        (r"D:\Apps\Shortcut\s.dat", 3 * GIB),
        (r"D:\Apps\Never\setup.exe", 2 * (1 << 20)),
    ] {
        fs.file(path, size);
    }
    let out = refresh(&db.conn().unwrap(), &fs, r"D:\", &RefreshOptions::default()).unwrap();
    assert!(out.completed);
}

pub fn fake_inputs() -> AppInputs {
    let mut hidden = program(
        "Hidden Part",
        Some(r"D:\Apps\Hidden"),
        Some(r#""D:\Apps\Hidden\u.exe""#),
    );
    hidden.hidden = true;
    AppInputs {
        programs: vec![
            program(
                "Big Tool",
                Some(r"D:\Apps\Big\"),
                Some(r#""D:\Apps\Big\unins000.exe""#),
            ),
            program(
                "Fresh",
                Some(r"D:\Apps\Fresh"),
                Some(r"D:\Apps\Fresh\uninstall.exe"),
            ),
            program(
                "Never Opened",
                Some(r"D:\Apps\Never"),
                Some(r"D:\Apps\Never\Uninstall Never.exe /x"),
            ),
            program("Tiny", Some(r"D:\Apps\Tiny"), Some(r"D:\Apps\Tiny\u.exe")),
            hidden,
            program("No Uninstaller", Some(r"D:\Apps\NoUninstall"), None),
            program(
                "Microsoft Visual Studio Code (User)",
                Some(r"D:\Apps\Code"),
                Some(r"D:\Apps\Code\unins000.exe"),
            ),
            program(
                "Contoso 365 Apps",
                Some(r"D:\Apps\Office"),
                Some(r"D:\Apps\Office\setup.exe /uninstall"),
            ),
            program(
                "Shortcut App",
                Some(r"D:\Apps\Shortcut"),
                Some(r"D:\Apps\Shortcut\u.exe"),
            ),
            program("Everything", Some(r"D:\Program Files"), Some(r"D:\x\u.exe")),
            program(
                "Not Indexed",
                Some(r"E:\Elsewhere"),
                Some(r"E:\Elsewhere\u.exe"),
            ),
            program("Steam", Some(r"D:\Steam"), Some(r"D:\Steam\uninstall.exe")),
            program(
                "Steam App 1001",
                Some(r"D:\Steam\steamapps\common\Old Game"),
                Some(r#""D:\Steam\steam.exe" steam://uninstall/1001"#),
            ),
        ],
        assist: vec![
            ran(r"D:\Apps\Big\big.exe", 200),
            ran(r"D:\Apps\Fresh\fresh.exe", 10),
            ran(r"D:\Steam\steam.exe", 300),
            ran(r"D:\Steam\steamapps\common\New Game\new.exe", 200),
            ran("Microsoft.VisualStudioCode", 2),
            ran("Microsoft.Office.WINWORD.EXE.15", 12),
            ran(
                r"{A77F5D77-2E2B-44C3-A6A2-ABA601054A51}\Shortcut App.lnk",
                20,
            ),
            ran(r"C:\Users\me\Downloads\setup.exe", 1),
        ],
        steam: vec![
            steam_game(1001, "Old Game", Some(100)),
            steam_game(1002, "New Game", Some(3)),
            steam_game(1003, "Unplayed", None),
        ],
    }
}

type AppRow<'a> = (&'a str, &'a str, &'a str, u64, Option<i64>, Option<&'a str>);

// Traces: ACT-APPS-RARELY-USED
#[test]
fn big_apps_not_run_for_90_days_are_listed_from_registry_userassist_and_steam() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("h.db")).unwrap();
    fake_disk(&db);
    let got = apps::rarely_used(&db.conn().unwrap(), &fake_inputs(), NOW).unwrap();
    let rows: Vec<AppRow> = got
        .iter()
        .map(|a: &RarelyUsedApp| {
            (
                a.name.as_str(),
                a.source,
                a.path.as_str(),
                a.size,
                a.last_run_ms,
                a.last_run_from.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "Old Game",
                "steam",
                r"D:\Steam\steamapps\common\Old Game",
                5 * GIB,
                Some(NOW - 100 * DAY),
                Some("Steam LastPlayed"),
            ),
            (
                "Never Opened",
                "registry",
                r"D:\Apps\Never",
                4 * GIB + 2 * (1 << 20),
                None,
                None,
            ),
            (
                "Big Tool",
                "registry",
                r"D:\Apps\Big",
                3 * GIB,
                Some(NOW - 200 * DAY),
                Some(r"last run of d:\apps\big\big.exe"),
            ),
            (
                "Unplayed",
                "steam",
                r"D:\Steam\steamapps\common\Unplayed",
                5 * GIB / 2,
                None,
                None,
            ),
        ]
    );
    assert_eq!(got[0].id, "steam-1001");
    let key = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Never Opened";
    let hash = blake3::hash(key.to_lowercase().as_bytes()).to_hex();
    assert_eq!(got[1].id, format!("reg-{}", &hash[..16]));
    let again = apps::rarely_used(&db.conn().unwrap(), &fake_inputs(), NOW).unwrap();
    assert_eq!(
        again.iter().map(|a| &a.id).collect::<Vec<_>>(),
        got.iter().map(|a| &a.id).collect::<Vec<_>>(),
        "ids are stable across calls"
    );
}

// Traces: ACT-APPS-RARELY-USED
#[test]
fn each_app_uninstalls_through_its_own_registration() {
    let all = apps::installed(&fake_inputs());
    let by_name = |n: &str| all.iter().find(|a| a.name == n).unwrap();
    assert_eq!(
        by_name("Old Game").uninstaller,
        Uninstaller::Url("steam://uninstall/1001".into())
    );
    assert_eq!(
        by_name("Never Opened").uninstaller,
        Uninstaller::Command {
            file: r"D:\Apps\Never\Uninstall Never.exe".into(),
            params: Some("/x".into()),
        }
    );
    assert!(!all.iter().any(|a| a.name == "Steam App 1001"));
    for gone in ["Hidden Part", "No Uninstaller", "Everything"] {
        assert!(!all.iter().any(|a| a.name == gone), "{gone}");
    }
}

// Traces: ACT-APPS-RARELY-USED, ACT-UNINSTALL
#[test]
fn a_bare_uninstaller_name_runs_only_from_the_system_folder() {
    let inputs = AppInputs {
        programs: vec![
            program(
                "Msi App",
                Some(r"D:\Apps\Msi"),
                Some("MsiExec.exe /X{11111111-2222-3333-4444-555555555555}"),
            ),
            program("Planted", Some(r"D:\Apps\Planted"), Some("unins000.exe /S")),
            program("Relative", Some(r"D:\Apps\Rel"), Some(r"bin\uninstall.exe")),
            program(
                "Drive Relative",
                Some(r"D:\Apps\Dr"),
                Some("D:uninstall.exe"),
            ),
            program(
                "Share",
                Some(r"D:\Apps\Share"),
                Some(r"\\server\apps\uninstall.exe /quiet"),
            ),
        ],
        ..AppInputs::default()
    };
    let all = apps::installed(&inputs);
    let by_name = |n: &str| all.iter().find(|a| a.name == n).map(|a| &a.uninstaller);
    let system = hoardini_win::system_directory().unwrap();
    assert_eq!(
        by_name("Msi App"),
        Some(&Uninstaller::Command {
            file: system.join("MsiExec.exe").to_string_lossy().into_owned(),
            params: Some("/X{11111111-2222-3333-4444-555555555555}".into()),
        })
    );
    assert_eq!(
        by_name("Share"),
        Some(&Uninstaller::Command {
            file: r"\\server\apps\uninstall.exe".into(),
            params: Some("/quiet".into()),
        })
    );
    for refused in ["Planted", "Relative", "Drive Relative"] {
        assert_eq!(by_name(refused), None, "{refused}");
    }
    assert_eq!(
        apps::program_path(r"C:\x\..\u.exe").as_deref(),
        Some(r"C:\x\..\u.exe")
    );
    assert_eq!(apps::program_path("..\\u.exe"), None);
}

// Traces: ACT-APPS-RARELY-USED
#[test]
fn uninstall_strings_split_into_program_and_arguments() {
    let s = |c: &str| split_command(c).map(|(f, p)| (f, p.unwrap_or_default()));
    assert_eq!(
        s(r#""C:\Program Files\App\unins000.exe" /SILENT"#),
        Some((
            r"C:\Program Files\App\unins000.exe".into(),
            "/SILENT".into()
        ))
    );
    assert_eq!(
        s(r"C:\Program Files\My App\Uninstall My App.exe --remove now"),
        Some((
            r"C:\Program Files\My App\Uninstall My App.exe".into(),
            "--remove now".into()
        ))
    );
    assert_eq!(
        s("MsiExec.exe /X{11111111-2222-3333-4444-555555555555}"),
        Some((
            "MsiExec.exe".into(),
            "/X{11111111-2222-3333-4444-555555555555}".into()
        ))
    );
    assert_eq!(
        s(r"C:\Tools\app.exe.old\run.exe"),
        Some((r"C:\Tools\app.exe.old\run.exe".into(), String::new()))
    );
    assert_eq!(s("   "), None);
    assert_eq!(s(r#""C:\unterminated.exe"#), None);
}

// Traces: ACT-APPS-RARELY-USED
#[test]
fn steam_library_folders_and_app_manifests_parse() {
    let modern = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"apps"
		{
			"228980"		"370616524"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
    assert_eq!(
        parse_library_folders(modern),
        vec![r"C:\Program Files (x86)\Steam", r"D:\SteamLibrary"]
    );
    let legacy =
        r#""LibraryFolders" { "TimeNextStatsReport" "1700000000" "1" "E:\\Games\\Steam" }"#;
    assert_eq!(parse_library_folders(legacy), vec![r"E:\Games\Steam"]);

    let acf = r#"
"AppState"
{
	"appid"		"1001"
	"name"		"Old Game"
	"installdir"		"Old Game"
	"LastPlayed"		"1791360000"
	"UserConfig"
	{
		"language"		"english"
	}
}"#;
    assert_eq!(
        parse_app_manifest(acf, "d:/steamlibrary"),
        Some(SteamApp {
            appid: 1001,
            name: "Old Game".into(),
            dir: r"D:\steamlibrary\steamapps\common\Old Game".into(),
            last_played_ms: Some(1_791_360_000_000),
        })
    );
    let never = acf.replace("1791360000", "0");
    assert_eq!(
        parse_app_manifest(&never, r"D:\SteamLibrary")
            .unwrap()
            .last_played_ms,
        None
    );
    assert_eq!(
        parse_app_manifest(r#""AppState" { "appid" "5" }"#, r"D:\S"),
        None
    );
}
