//! Calling Tawny's own Kotlin classes (`android/localstreams`) from Rust.
//!
//! Each plugin is constructed once with the Application and kept; its methods
//! take strings and answer a string (JSON, by convention), which keeps the JNI
//! here to one shape.

use jni::{
    JavaVM,
    objects::{GlobalRef, JClass, JObject, JString, JValue},
};

/// Declared for its side effect: this is what tells `dx` to build and bundle
/// the Kotlin module. The calls go through JNI below, like g3-native-plugins'.
#[manganis::ffi("android/localstreams")]
unsafe extern "Kotlin" {
    pub type LocalStreamsPlugin;
}

/// A Kotlin plugin, constructed once with the Application.
pub struct Plugin {
    instance: GlobalRef,
    vm: JavaVM,
    name: &'static str,
}

impl Plugin {
    /// Constructs `class` through the app's own class loader: from a thread
    /// attached from native code, `FindClass` only sees the system's classes.
    ///
    /// `ndk_context` holds the Application, not the Activity, and that is what
    /// the plugin is built with: it only needs the application context, and a
    /// cached plugin holding an Activity would outlive activity recreation
    /// (rotation, dark mode). Anything that needs the Activity itself goes
    /// through `manganis::android::with_activity`.
    pub fn construct(class: &str, name: &'static str) -> Result<Self, String> {
        let android = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(android.vm().cast()) }
            .map_err(|error| format!("Could not reach the Android VM: {error}"))?;
        let mut env = vm
            .attach_current_thread_permanently()
            .map_err(|error| format!("Could not attach to the Android VM: {error}"))?;
        let application = unsafe { JObject::from_raw(android.context().cast()) };
        let loader = env
            .call_method(
                &application,
                "getClassLoader",
                "()Ljava/lang/ClassLoader;",
                &[],
            )
            .and_then(|value| value.l())
            .map_err(|error| format!("Could not find the app's class loader: {error}"))?;
        let class_name = JObject::from(
            env.new_string(class)
                .map_err(|error| format!("Could not name the {name}: {error}"))?,
        );
        let class = env
            .call_method(
                loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&class_name)],
            )
            .and_then(|value| value.l())
            .map_err(|error| format!("The {name} is not in this build: {error}"))?;
        let instance = env
            .new_object(
                JClass::from(class),
                "(Landroid/content/Context;)V",
                &[JValue::Object(&application)],
            )
            .map_err(|error| format!("Could not start the {name}: {error}"))?;
        let instance = env
            .new_global_ref(instance)
            .map_err(|error| format!("Could not keep the {name}: {error}"))?;
        // The Application reference stays the host's: a JObject releases
        // nothing when it goes out of scope.
        Ok(Self { instance, vm, name })
    }

    /// Calls `method(String, ...): String` with `arguments`.
    pub fn call(&self, method: &str, arguments: &[&str]) -> Result<String, String> {
        let name = self.name;
        let mut env = self
            .vm
            .attach_current_thread()
            .map_err(|error| format!("Could not reach the {name}: {error}"))?;
        let mut strings = Vec::with_capacity(arguments.len());
        for argument in arguments {
            strings.push(JObject::from(env.new_string(argument).map_err(
                |error| format!("Could not pass a value to the {name}: {error}"),
            )?));
        }
        let values: Vec<JValue> = strings.iter().map(JValue::Object).collect();
        let signature = format!(
            "({})Ljava/lang/String;",
            "Ljava/lang/String;".repeat(arguments.len())
        );
        let value = env
            .call_method(self.instance.as_obj(), method, &signature, &values)
            .and_then(|value| value.l())
            .map_err(|error| format!("The {name} failed: {error}"))?;
        let text = JString::from(value);
        let answer = env
            .get_string(&text)
            .map_err(|error| format!("The {name} answered unreadable text: {error}"))?;
        Ok(answer.into())
    }
}
