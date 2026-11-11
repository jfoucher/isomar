# R8 shrinks the libraries (Compose's extended icons are most of them) and leaves the app's own classes as they
# are: the engine calls back into them over JNI by name (SceneSink and the rest in Native.kt), and
# kotlinx.serialization finds the records' generated serializers by name.
-keep class io.github.isomar.** { *; }
-keepclasseswithmembernames class * {
    native <methods>;
}
