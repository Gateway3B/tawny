// Resolving playback on the phone: yt-dlp in an embedded Python (Chaquopy),
// with YouTube's JavaScript challenges solved in Android's V8 sandbox. See
// docs/on-device-playback.md.
//
// Bundled into the app by `#[manganis::ffi("android/localstreams")]` in
// src/local_streams/bridge.rs. Only the Android build includes it.

import org.gradle.api.tasks.bundling.AbstractArchiveTask

buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        // The generated root project has no plugin management block, so the
        // plugin is put on this module's own build classpath.
        classpath("com.chaquo.python:gradle:17.1.0")
    }
}

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

apply(plugin = "com.chaquo.python")

android {
    namespace = "net.gateway3.tawny.localstreams"
    compileSdk = 35

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
        // Phones, and the x86_64 emulator. Chaquopy ships a Python per ABI.
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        getByName("release") { isMinifyEnabled = false }
        getByName("debug") { isMinifyEnabled = false }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions { jvmTarget = "17" }
}

extensions.configure<com.chaquo.python.ChaquopyExtension>("chaquopy") {
    defaultConfig {
        version = "3.13"
        pip {
            // Pinned to what the server's extractor sidecar runs, so the two
            // resolve alike. `tawny_local.py` reaches into its internals.
            install("yt-dlp==2026.8.19")
            install("yt-dlp-ejs==0.8.0")
            install("certifi")
        }
    }
}

tasks.withType<AbstractArchiveTask>().configureEach {
    archiveBaseName.set("tawny-local-streams")
}

dependencies {
    implementation("androidx.javascriptengine:javascriptengine:1.0.0")
}
