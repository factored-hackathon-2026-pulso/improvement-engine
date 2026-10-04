//! Optional static console: files under `static_dir` are served for non-API GETs, `/config.json` can be overridden
//! (so the console points its http provider at this server without a proxy), `index.html` is the SPA fallback,
//! and nothing outside the directory is reachable.
use debug_api::{App, Config, Req, Store};
use std::collections::HashMap;
use std::sync::Arc;

fn get(app: &App, path: &str) -> debug_api::Resp {
    app.handle(&Req { method: "GET".into(), path: path.into(), query: String::new(), headers: HashMap::new(), body: vec![] })
}
fn dist() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("dapi-static-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("assets")).unwrap();
    std::fs::write(d.join("index.html"), "<html>console</html>").unwrap();
    std::fs::write(d.join("config.json"), r#"{"provider":"fixture"}"#).unwrap();
    std::fs::write(d.join("assets/app.js"), "console.log(1)").unwrap();
    std::fs::write(d.parent().unwrap().join("dapi-secret.txt"), "secret").unwrap();
    d
}
fn app(cfg: Config) -> App {
    App::new(Arc::new(Store::memory()), cfg)
}
fn ctype(r: &debug_api::Resp) -> String {
    r.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type")).map(|(_, v)| v.clone()).unwrap_or_default()
}

#[test]
fn off_by_default() {
    assert_eq!(get(&app(Config::default()), "/").status, 404);
}

#[test]
fn serves_index_assets_with_types_and_config_override() {
    let a = app(Config { static_dir: Some(dist()), config_json: Some(r#"{"provider":"http"}"#.into()), ..Config::default() });
    let i = get(&a, "/");
    assert_eq!((i.status, ctype(&i).as_str(), i.body.as_slice()), (200, "text/html; charset=utf-8", &b"<html>console</html>"[..]));
    let js = get(&a, "/assets/app.js");
    assert_eq!((js.status, ctype(&js).as_str()), (200, "text/javascript; charset=utf-8"));
    assert_eq!(get(&a, "/config.json").body, br#"{"provider":"http"}"#);
    assert_eq!(get(&a, "/some/spa/route").body, b"<html>console</html>", "SPA fallback");
    assert_eq!(get(&a, "/assets/missing.js").status, 404, "a missing asset is not index.html");
    assert_eq!(get(&a, "/healthz").status, 200, "API routes win");
    assert_eq!(get(&a, "/internal/v1/debug/nope").status, 404);
}

#[test]
fn never_leaves_the_directory() {
    let a = app(Config { static_dir: Some(dist()), ..Config::default() });
    for p in ["/../dapi-secret.txt", "/%2e%2e/dapi-secret.txt", "/assets/../../dapi-secret.txt", "/..%5cdapi-secret.txt"] {
        let r = get(&a, p);
        assert_ne!(r.body, b"secret", "{p}");
    }
}

#[test]
fn device_names_and_symlinks_do_not_escape() {
    let d = dist();
    let a = app(Config { static_dir: Some(d.clone()), ..Config::default() });
    for p in ["/nul", "/con", "/aux.txt", "/COM1", "/NUL.js", "//dapi-secret.txt", "/C:/Windows/win.ini", "/index.html::$DATA"] {
        let r = get(&a, p);
        assert_ne!(r.body, b"secret", "{p}");
        assert!(r.status == 404 || r.body == b"<html>console</html>", "{p}: {}", r.status);
    }
    // A symlink inside the directory that points outside it is never followed (skipped when the OS refuses to create one).
    let link = d.join("leak.txt");
    let target = d.parent().unwrap().join("dapi-secret.txt");
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(&target, &link).is_ok();
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&target, &link).is_ok();
    if made {
        assert_ne!(get(&a, "/leak.txt").body, b"secret", "symlink escape");
    }
}
