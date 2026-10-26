# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile

# The Rust side finds these classes by name and calls into them over JNI,
# which R8 cannot see.
-keep class dev.kemalarslan.carrychat.CarryChatService { *; }
-keep class dev.kemalarslan.carrychat.CarryChatService$Companion { *; }
-keep class dev.kemalarslan.carrychat.Ble { *; }
-keep class dev.kemalarslan.carrychat.Core { *; }

# The page calls these through the WebView's JavaScript bridge.
-keepattributes JavascriptInterface
-keepclassmembers class dev.kemalarslan.carrychat.MainActivity$Bars {
    @android.webkit.JavascriptInterface <methods>;
}
