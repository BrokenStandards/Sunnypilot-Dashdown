# JNA + uniffi-generated bindings load native code by reflection; keep them.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class uniffi.dashdown_core.** { *; }
