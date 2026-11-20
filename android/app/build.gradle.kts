plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "app.dread.client"
    compileSdk = 36

    defaultConfig {
        applicationId = "app.dread.client"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "1.0.0"
    }

    signingConfigs {
        create("release") {
            // A local release key is created by ../build-apk.sh when none is configured.
            val ks = file(System.getenv("DREAD_KEYSTORE") ?: "../dread-release.jks")
            storeFile = ks
            storePassword = System.getenv("DREAD_KEYSTORE_PASSWORD") ?: "dread-local"
            keyAlias = System.getenv("DREAD_KEY_ALIAS") ?: "dread"
            keyPassword = System.getenv("DREAD_KEY_PASSWORD") ?: "dread-local"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.getByName("release")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    packaging {
        resources.excludes += "META-INF/*"
    }
}

dependencies {
    implementation("com.google.zxing:core:3.5.4")
}
