pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // usb-serial-for-android (USB NMEA adapters) is published there only.
        maven("https://jitpack.io") { content { includeGroup("com.github.mik3y") } }
    }
}

rootProject.name = "Isomar"

include(":app")
