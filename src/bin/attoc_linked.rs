//! Native executable linked-source driver.
//!
//! Build operator supplies an explicit, local source catalogue. No GitHub
//! secrets, remote imports or precompiled guest metadata are loaded here.
//! Source files are read from inside a canonical root, rejecting traversal
//! and symbolic-link components. This is not a supply-chain signature verifier.
use std::path::{Component,Path,PathBuf};
use std::process::ExitCode;
use std::io::{Read,Write};
use std::collections::BTreeMap;
use atto_compiler::source_link::{compile_linked,compile_pinned,SourceUnit,PinnedSourceUnit};

struct Owned {
    package:String, namespace:String, module:String, source:String,
    key:String,
}
fn error(message: &str) -> ExitCode {
    eprintln!("attoc-linked: {message}");
    ExitCode::from(2)
}
fn run() -> Result<Vec<u8>,String> {
    let mut args=std::env::args().skip(1);
    let mut root:Option<String>=None;
    let mut entry:Option<String>=None;
    let mut files=Vec::<(String,String)>::new();
    let mut pins=BTreeMap::<String,(String,String)>::new();
    let mut allow_unpinned=false;
    while let Some(flag)=args.next() {
        if flag=="--allow-unpinned-local" {
            if allow_unpinned {return Err("duplicate --allow-unpinned-local".into());}
            allow_unpinned=true;
            continue;
        }
        let arg=args.next().ok_or_else(||
            format!("{flag} requires a value"))?;
        match flag.as_str() {
            "--root" if root.is_none() => root=Some(arg),
            "--entry" if entry.is_none() => entry=Some(arg),
            "--unit" => files.push(arg.split_once('=')
                .map(|(k,v)|(k.to_string(),v.to_string()))
                .ok_or("--unit requires package/namespace/module=relative/path.atto")?),
            "--pin" => {
                let (id,value)=arg.split_once('=')
                    .ok_or("--pin requires package/namespace/module=VERSION@SHA256")?;
                let (version,digest)=value.split_once('@')
                    .ok_or("--pin requires VERSION@SHA256")?;
                if pins.insert(id.into(),(version.into(),digest.into())).is_some() {
                    return Err("duplicate pinned unit identity".into());
                }
            }
            _ => return Err("usage: attoc-linked --root DIR --entry PACKAGE/NAMESPACE/MODULE --unit PACKAGE/NAMESPACE/MODULE=RELATIVE.atto --pin PACKAGE/NAMESPACE/MODULE=VERSION@SHA256 [--unit/--pin ...]; local legacy mode: --allow-unpinned-local".into())
        }
    }
    let root=root.ok_or("--root is required")?;
    let root=std::fs::canonicalize(&root)
        .map_err(|e|format!("invalid build root: {e}"))?;
    if !root.is_dir() {return Err("build root is not a directory".into());}
    let entry=entry.ok_or("--entry is required")?;
    if files.is_empty() || files.len()>32 {
        return Err("source catalogue must contain between 1 and 32 units".into());
    }
    let mut owned=Vec::new();
    let mut found_entry=None;
    for (idx,(identity,relative)) in files.iter().enumerate() {
        if identity==&entry {
            if found_entry.replace(idx).is_some() {
                return Err("duplicate root entry identity".into());
            }
        }
        let parts=identity.split('/').collect::<Vec<_>>();
        if parts.len()!=3 || parts.iter().any(|s|s.is_empty()) {
            return Err(format!("invalid unit identity: {identity}"));
        }
        let rel=Path::new(relative);
        if rel.as_os_str().is_empty() || !rel.components()
            .all(|c|matches!(c,Component::Normal(_))) {
            return Err(format!("unsafe relative source path: {relative}"));
        }
        let mut absolute=PathBuf::from(&root);
        for component in rel.components() {
            absolute.push(component.as_os_str());
            let meta=std::fs::symlink_metadata(&absolute)
                .map_err(|e|format!("missing source component {relative}: {e}"))?;
            if meta.file_type().is_symlink() {
                return Err(format!("symlink source components are unsupported: {relative}"));
            }
        }
        let mut options=std::fs::OpenOptions::new();
        options.read(true);
        // O_NOFOLLOW makes final-component symlink replacement fail even if
        // an attacker switches it between symlink_metadata and open().
        #[cfg(target_os="linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(0o400000);
        }
        #[cfg(target_os="macos")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(0x100);
        }
        let handle=options.open(&absolute)
            .map_err(|e|format!("cannot open pinned source {relative}: {e}"))?;
        let meta=handle.metadata()
            .map_err(|e|format!("cannot stat opened source {relative}: {e}"))?;
        if !meta.is_file() || meta.len() as usize>atto_compiler::SOURCE_CAP {
            return Err(format!("source is not a bounded regular file: {relative}"));
        }
        let mut source=String::new();
        handle.take((atto_compiler::SOURCE_CAP+1) as u64)
            .read_to_string(&mut source)
            .map_err(|e|format!("source is not readable UTF-8 {relative}: {e}"))?;
        if source.len()>atto_compiler::SOURCE_CAP {
            return Err(format!("opened source exceeds compiler source budget: {relative}"));
        }
        owned.push(Owned {package:parts[0].into(),namespace:parts[1].into(),
            module:parts[2].into(),source,key:identity.clone()});
    }
    let entry=found_entry.ok_or("entry identity has no catalogued source")?;
    let units:Vec<SourceUnit<'_>>=owned.iter().map(|u|SourceUnit {
        package:&u.package,namespace:&u.namespace,module:&u.module,source:&u.source
    }).collect();
    if allow_unpinned {
        if !pins.is_empty() {
            return Err("--allow-unpinned-local cannot be combined with --pin".into());
        }
        return compile_linked(&units,entry).map_err(|e|format!("{e}"));
    }
    if pins.len()!=units.len() {
        return Err("every input unit requires a pinned package version and SHA-256; use --allow-unpinned-local only for development".into());
    }
    let mut pinned=Vec::with_capacity(units.len());
    for (unit,owned_unit) in units.into_iter().zip(&owned) {
        let (version,digest)=pins.get(&owned_unit.key)
            .ok_or_else(||format!("missing source pin for {}",owned_unit.key))?;
        pinned.push(PinnedSourceUnit {
            unit,package_version:version,source_sha256:digest,
        });
    }
    compile_pinned(&pinned,entry).map_err(|e|format!("{e}"))
}
fn main() -> ExitCode {
    match run() {
        Ok(bytes) => {
            if let Err(e)=std::io::stdout().write_all(&bytes) {
                return error(&format!("cannot write binary output: {e}"));
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("attoc-linked: {message}");
            ExitCode::from(1)
        }
    }
}
