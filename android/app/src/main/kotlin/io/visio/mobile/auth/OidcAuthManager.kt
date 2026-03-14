package io.visio.mobile.auth

import android.content.Context
import android.content.Intent
import androidx.activity.result.ActivityResultLauncher
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

class OidcAuthManager(context: Context) {
    private val masterKey =
        MasterKey.Builder(context)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()

    private val prefs = try {
        EncryptedSharedPreferences.create(
            context,
            "visio_auth",
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    } catch (_: Exception) {
        context.deleteSharedPreferences("visio_auth")
        EncryptedSharedPreferences.create(
            context,
            "visio_auth",
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    }

    fun launchOidcFlow(
        launcher: ActivityResultLauncher<Intent>,
        context: Context,
        meetInstance: String,
    ) {
        val intent = Intent(context, OidcLoginActivity::class.java)
        intent.putExtra(OidcLoginActivity.EXTRA_MEET_INSTANCE, meetInstance)
        launcher.launch(intent)
    }

    fun saveCookie(cookie: String) {
        prefs.edit().putString("sessionid", cookie).apply()
    }

    fun getSavedCookie(): String? {
        return prefs.getString("sessionid", null)
    }

    fun clearCookie() {
        prefs.edit().remove("sessionid").apply()
    }
}
