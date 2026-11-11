//! The app's side: find the backend library, load it, and start it.

use std::{
    ffi::{CString, c_char},
    path::PathBuf,
    sync::OnceLock,
};

use super::ABI_VERSION;

type AbiFn = extern "C" fn() -> u32;
type StartFn = unsafe extern "C" fn(*const c_char) -> u16;

/// Start the backend in this process, once, and return its address.
///
/// Later calls return the first answer: the library stays loaded and its
/// server keeps running for the life of the app.
pub fn start() -> Result<String, String> {
    static STARTED: OnceLock<Result<String, String>> = OnceLock::new();
    STARTED.get_or_init(load_and_start).clone()
}

fn load_and_start() -> Result<String, String> {
    let data_dir = data_dir().ok_or("This device has no folder for app data.")?;
    std::fs::create_dir_all(&data_dir)
        .map_err(|error| format!("Could not create {}: {error}", data_dir.display()))?;
    let library = open_library()?;
    // SAFETY: both symbols are defined by `serve` with exactly these
    // signatures, and the ABI version is checked before the second is used.
    let port = unsafe {
        let abi = library
            .get::<AbiFn>(b"tawny_backend_abi\0")
            .map_err(|error| format!("The backend library is not Tawny's: {error}"))?;
        if abi() != ABI_VERSION {
            return Err("The backend library belongs to another version of Tawny.".into());
        }
        let start = library
            .get::<StartFn>(b"tawny_backend_start\0")
            .map_err(|error| format!("The backend library is not Tawny's: {error}"))?;
        let path = CString::new(data_dir.to_string_lossy().as_bytes())
            .map_err(|_| "The app's data folder has an unusable name.".to_string())?;
        start(path.as_ptr())
    };
    // Never unloaded: the backend's threads run code from it until the app
    // exits.
    std::mem::forget(library);
    if port == 0 {
        return Err("The backend could not start.".into());
    }
    Ok(format!("http://127.0.0.1:{port}"))
}

/// Android finds the library by name among the app's own native libraries.
#[cfg(target_os = "android")]
fn open_library() -> Result<libloading::Library, String> {
    // SAFETY: loading runs the library's initializers, which are Rust's own
    // and the server functions' registration, and nothing else.
    unsafe { libloading::Library::new("libtawny_backend.so") }
        .map_err(|error| format!("This build of Tawny has no device backend: {error}"))
}

/// Desktop looks next to the executable, where a bundle puts it, then at
/// `TAWNY_BACKEND_LIBRARY`, which development builds set (`just dev-desktop`).
#[cfg(not(target_os = "android"))]
fn open_library() -> Result<libloading::Library, String> {
    let name = libloading::library_filename("tawny_backend");
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    {
        candidates.push(dir.join(&name));
        // A macOS bundle keeps libraries beside, not inside, MacOS/.
        candidates.push(dir.join("..").join("Frameworks").join(&name));
    }
    if let Some(path) = std::env::var_os("TAWNY_BACKEND_LIBRARY") {
        candidates.push(PathBuf::from(path));
    }
    let path = candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or("This build of Tawny has no device backend.")?;
    // SAFETY: as on Android.
    unsafe { libloading::Library::new(&path) }
        .map_err(|error| format!("Could not load {}: {error}", path.display()))
}

/// Where the library lives: the app's private files on Android, the user's
/// local data folder elsewhere. Not a cache directory, which the system may
/// empty: this is the only copy of the viewer's library.
#[cfg(target_os = "android")]
pub fn data_dir() -> Option<PathBuf> {
    use jni::objects::{JObject, JString};

    let context = ndk_context::android_context();
    // SAFETY: `ndk_context` hands out the process's live JavaVM and
    // Application context, which outlive this call.
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let application = unsafe { JObject::from_raw(context.context().cast()) };
    let dir = env
        .call_method(&application, "getFilesDir", "()Ljava/io/File;", &[])
        .ok()?
        .l()
        .ok()?;
    let path = env
        .call_method(&dir, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    let path: String = env.get_string(&JString::from(path)).ok()?.into();
    Some(PathBuf::from(path).join("library"))
}

#[cfg(not(target_os = "android"))]
pub fn data_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("Tawny").join("device"))
}
