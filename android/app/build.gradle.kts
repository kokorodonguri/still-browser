plugins {
    id("com.android.application")
}

// Windows sets the version for every platform (windows/launcher/Cargo.toml); raise it there for each release.
val stillVersion: String = Regex("""(?m)^version = "([^"]+)"""")
    .find(providers.fileContents(rootProject.layout.projectDirectory.file("../windows/launcher/Cargo.toml")).asText.get())!!
    .groupValues[1]
// Installed copies update only to a higher versionCode: 0.5.1 is 501, so minor and patch stay below 100.
val stillVersionCode: Int = stillVersion.split('.').map(String::toInt).let { (major, minor, patch) ->
    require(minor < 100 && patch < 100) { "versionCode needs minor and patch below 100: $stillVersion" }
    major * 10000 + minor * 100 + patch
}

android {
    namespace = "app.still.browser"
    // GeckoView 158 compiles against Android 17 QPR2 (API 37.2).
    compileSdk {
        version = release(37) { minorApiLevel = 2 }
    }

    defaultConfig {
        applicationId = "app.still.browser"
        minSdk = 29
        targetSdk = 36
        versionCode = stillVersionCode
        versionName = stillVersion
        // Where Updater.kt reads update.json (written by site/build.py). Tests point it elsewhere with -PstillUpdateUrl=….
        resValue("string", "update_url", providers.gradleProperty("stillUpdateUrl").getOrElse("https://still.dongurihub.com/update.json"))
        // GeckoView is native code for each processor; one keeps the APK near 80 MB.
        // Phones are arm64-v8a. Emulator tests build with -PstillAbis=x86_64.
        ndk {
            abiFilters += providers.gradleProperty("stillAbis").getOrElse("arm64-v8a").split(',')
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            // No release key exists yet; the debug key lets the APK be installed for testing.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    buildFeatures {
        resValues = true
    }

    packaging {
        // Compressed native code: GeckoView's ~200 MB of libraries download as ~60 MB.
        // Android unpacks them once at install.
        jniLibs {
            useLegacyPackaging = true
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    testOptions {
        unitTests.isReturnDefaultValues = false
    }
}

dependencies {
    // Firefox's engine. Raise with each Firefox release (https://maven.mozilla.org/maven2/org/mozilla/geckoview/geckoview/).
    implementation("org.mozilla.geckoview:geckoview:158.0.20261007201254")
    // Android's own org.json is a stub on the JVM; tests use the reference implementation.
    testImplementation("org.json:json:20260814")
    testImplementation("junit:junit:4.13.2")
}
