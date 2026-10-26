# Rust reaches LocalStreamsPlugin through JNI and Python reaches JsSandbox
# through Chaquopy, both by name: nothing in Java or Kotlin refers to them, so
# without this R8 strips or renames them out of a release build.
-keep class net.gateway3.tawny.localstreams.** { *; }
