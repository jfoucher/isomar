plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// The release key (scripts/release.sh passes these; it keeps the key in ~/.config/isomar-release). Every APK
// published must be signed with the same key, or installed copies refuse the update. Without them a release
// build is unsigned.
val releaseKeystore: String? = providers.environmentVariable("ISOMAR_ANDROID_KEYSTORE").orNull

android {
    namespace = "io.github.isomar"
    compileSdk = 37
    // The NDK the Rust library and its C libraries are built with (scripts/android-build.sh).
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "eu.sixpx.isomar"
        // bionic has iconv from API 28, which glib needs.
        minSdk = 28
        targetSdk = 36
        // MAJOR*10000 + MINOR*100 + PATCH of versionName; it must grow with every APK published.
        versionCode = 700
        versionName = "0.7.0"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    signingConfigs {
        if (releaseKeystore != null) {
            create("release") {
                storeFile = file(releaseKeystore)
                storePassword = providers.environmentVariable("ISOMAR_ANDROID_KEYSTORE_PASSWORD").orNull
                keyAlias = providers.environmentVariable("ISOMAR_ANDROID_KEY_ALIAS").orNull ?: "isomar"
                // A PKCS12 keystore has one password for the store and the key.
                keyPassword = providers.environmentVariable("ISOMAR_ANDROID_KEYSTORE_PASSWORD").orNull
            }
        }
    }

    buildTypes {
        release {
            // R8 shrinks the libraries (the APK is downloaded from the website); proguard-rules.pro keeps the app.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (releaseKeystore != null) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    packaging {
        jniLibs {
            // Compressed in the APK (a third of the download), extracted when installed.
            useLegacyPackaging = true
            // The engine's library is stripped by cargo's release profile settings; keep it as built.
            keepDebugSymbols += "**/libisomar_android.so"
        }
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons.extended)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.androidx.window)
    implementation(libs.androidx.lifecycle.service)
    implementation(libs.usb.serial)
}
