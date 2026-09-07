import javax.inject.Inject
import org.gradle.process.ExecOperations

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
}

val repositoryRoot = rootProject.layout.projectDirectory.dir("../..")
val workspaceManifest = repositoryRoot.file("Cargo.toml").asFile.readText()
val workspaceProductVersion = Regex("(?m)^version = \"([^\"]+)\"$")
    .find(workspaceManifest)?.groupValues?.get(1) ?: error("Missing workspace version")
val androidVersionCode = providers.gradleProperty("ztermVersionCode").orElse(providers.exec {
    commandLine("python3", repositoryRoot.file("tools/android/release.py").asFile,
        "version-code", workspaceProductVersion)
}.standardOutput.asText.map { it.trim() }).get().toInt()
val releaseSourceCommit = providers.environmentVariable("ZTERM_SOURCE_COMMIT").orElse("development")
val androidSourceCommit = providers.environmentVariable("ZTERM_ANDROID_SOURCE_COMMIT").orElse(releaseSourceCommit)

android {
    namespace = "io.github.leonfox28.zterm"
    compileSdk = 36
    ndkVersion = "28.2.13676358"
    testBuildType = providers.gradleProperty("ztermTestBuildType").orElse("debug").get()
    defaultConfig {
        applicationId = "io.github.leonfox28.zterm"
        minSdk = 26
        targetSdk = 36
        versionCode = androidVersionCode
        versionName = workspaceProductVersion
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += "arm64-v8a" }
    }
    buildTypes {
        getByName("debug") {
            applicationIdSuffix = ".dev"
        }
    }
    bundle { language { enableSplit = false } }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    packaging {
        jniLibs.useLegacyPackaging = false
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
}

abstract class BuildRust : DefaultTask() {
    @get:Inject abstract val execOperations: ExecOperations
    @get:Internal abstract val repository: DirectoryProperty
    @get:Internal abstract val sdk: DirectoryProperty
    @get:InputFiles @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val rustInputs: ConfigurableFileCollection
    @get:Input abstract val ndkVersion: Property<String>
    @get:Input abstract val sourceCommit: Property<String>
    @get:OutputDirectory abstract val kotlinOutput: DirectoryProperty
    @get:OutputDirectory abstract val nativeOutput: DirectoryProperty

    @TaskAction fun build() {
        execOperations.exec {
            workingDir(repository)
            commandLine("python3", "tools/android/native.py",
                "--sdk", sdk.get().asFile.absolutePath,
                "--ndk-version", ndkVersion.get(),
                "--kotlin-out", kotlinOutput.get().asFile.absolutePath,
                "--jni-out", nativeOutput.get().asFile.absolutePath)
        }
    }
}

abstract class BuildIdentity : DefaultTask() {
    @get:Input abstract val productVersion: Property<String>
    @get:Input abstract val versionCode: Property<Int>
    @get:Input abstract val sourceCommit: Property<String>
    @get:OutputDirectory abstract val output: DirectoryProperty

    @TaskAction fun generate() {
        val directory = output.get().asFile
        directory.mkdirs()
        directory.resolve("zterm-build.json").writeText(groovy.json.JsonOutput.toJson(mapOf(
            "version" to productVersion.get(), "version_code" to versionCode.get(),
            "source_commit" to sourceCommit.get())))
    }
}

val buildIdentity = tasks.register<BuildIdentity>("buildIdentity") {
    productVersion.set(workspaceProductVersion)
    versionCode.set(androidVersionCode)
    sourceCommit.set(androidSourceCommit)
    output.set(layout.buildDirectory.dir("generated/buildIdentity/assets"))
}

val buildRust = tasks.register<BuildRust>("buildRust") {
    repository.set(repositoryRoot)
    sdk.set(androidComponents.sdkComponents.sdkDirectory)
    ndkVersion.set(android.ndkVersion)
    sourceCommit.set(releaseSourceCommit)
    rustInputs.from(fileTree(repositoryRoot) {
        include("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/**")
        include("crates/**/Cargo.toml", "crates/**/build.rs", "crates/**/src/**", "proto/**")
        include("crates/android/uniffi.toml", "tools/uniffi-bindgen/**", "tools/android/native.py")
        exclude("**/target/**", "**/__pycache__/**")
    })
    kotlinOutput.set(layout.buildDirectory.dir("generated/rust/kotlin"))
    nativeOutput.set(layout.buildDirectory.dir("generated/rust/jniLibs"))
}
androidComponents.onVariants { variant ->
    variant.sources.assets?.addGeneratedSourceDirectory(buildIdentity, BuildIdentity::output)
    variant.sources.java?.addGeneratedSourceDirectory(buildRust, BuildRust::kotlinOutput)
    variant.sources.jniLibs?.addGeneratedSourceDirectory(buildRust, BuildRust::nativeOutput)
}

dependencyLocking { lockAllConfigurations() }

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.material)
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.compose)
    implementation(libs.coroutines.android)
    implementation("net.java.dev.jna:jna:${libs.versions.jna.get()}@aar")
    implementation(libs.camera.camera2)
    implementation(libs.camera.lifecycle)
    implementation(libs.camera.view)
    implementation(libs.barcode)
    debugImplementation(libs.compose.tooling)
    debugImplementation(libs.compose.test.manifest)
    testImplementation(libs.junit)
    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.test)
    androidTestImplementation(libs.test.runner)
    androidTestImplementation(libs.test.junit)
}
