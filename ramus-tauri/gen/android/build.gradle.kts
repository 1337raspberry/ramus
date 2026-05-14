buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:8.11.0")
        // Bumped to 2.2.0 alongside the libmpv migration —
        // `dev.jdtech.mpv:libmpv:1.0.0` ships Kotlin 2.2 metadata, which
        // the 1.9.x compiler can't read (errors with "actual metadata
        // version is 2.2.0, compiler can read up to 2.0.0").
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.2.0")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

tasks.register("clean").configure {
    delete("build")
}

