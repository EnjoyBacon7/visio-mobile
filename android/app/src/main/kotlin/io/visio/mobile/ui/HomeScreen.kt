package io.visio.mobile.ui

import android.util.Log
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.clickable
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import android.app.Activity
import android.content.Intent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import io.visio.mobile.R
import io.visio.mobile.VisioManager
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Public
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Smartphone
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import io.visio.mobile.ui.i18n.Strings
import io.visio.mobile.ui.theme.VisioColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.visio.RoomValidationResult

private const val TAG = "HomeScreen"

@Composable
fun HomeScreen(
    onJoin: (roomUrl: String, username: String) -> Unit,
    onSettings: () -> Unit,
) {
    val context = LocalContext.current
    val oidcLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        if (result.resultCode == Activity.RESULT_OK) {
            val cookie = result.data?.getStringExtra("sessionid")
            val meetInstance = result.data?.getStringExtra("meet_instance")
            if (cookie != null && meetInstance != null) {
                VisioManager.onAuthCookieReceived(cookie, meetInstance)
            }
        }
    }
    var roomUrl by remember { mutableStateOf("") }
    var username by remember { mutableStateOf("") }
    val lang = VisioManager.currentLang
    val isDark = VisioManager.currentTheme == "dark"
    var roomStatus by remember { mutableStateOf("idle") }
    val slugRegex = remember { Regex("^[a-z]{3}-[a-z]{4}-[a-z]{3}$") }
    var meetInstances by remember { mutableStateOf(listOf<String>()) }
    var showServerPicker by remember { mutableStateOf(false) }
    var showCreateRoom by remember { mutableStateOf(false) }
    var customServer by remember { mutableStateOf("") }

    // Load meet instances from settings
    LaunchedEffect(Unit) {
        try {
            meetInstances = VisioManager.client.getMeetInstances()
        } catch (e: Exception) {
            Log.e(TAG, "Failed to load meet instances", e)
        }
    }

    // Resolve full URL: if input is just a slug, prefix with first configured server
    fun resolveRoomUrl(input: String): String {
        val trimmed = input.trim()
        return if (slugRegex.matches(trimmed) && meetInstances.isNotEmpty()) {
            "https://${meetInstances.first()}/$trimmed"
        } else {
            trimmed
        }
    }

    LaunchedEffect(VisioManager.pendingDeepLink) {
        val link = VisioManager.pendingDeepLink
        if (link != null) {
            roomUrl = link
            VisioManager.pendingDeepLink = null
        }
    }

    LaunchedEffect(roomUrl) {
        val resolved = resolveRoomUrl(roomUrl)
        val trimmed = resolved.trimEnd('/')
        val candidate = if ('/' in trimmed) trimmed.substringAfterLast('/') else trimmed
        if (!slugRegex.matches(candidate)) {
            roomStatus = "idle"
            return@LaunchedEffect
        }
        roomStatus = "checking"
        delay(500)
        try {
            val result =
                withContext(Dispatchers.IO) {
                    VisioManager.client.validateRoom(resolved, username.trim().ifEmpty { null })
                }
            roomStatus =
                when (result) {
                    is RoomValidationResult.Valid -> "valid"
                    is RoomValidationResult.NotFound -> "not_found"
                    is RoomValidationResult.InvalidFormat -> "idle"
                    is RoomValidationResult.NetworkError -> "error"
                }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to validate room URL", e)
            roomStatus = "error"
        }
    }

    // Pre-fill display name from VisioManager observable state
    LaunchedEffect(VisioManager.displayName) {
        val name = VisioManager.displayName
        if (name.isNotBlank() && username.isEmpty()) {
            username = name
        }
    }

    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.background)
                .statusBarsPadding()
                .navigationBarsPadding()
                .imePadding()
                .verticalScroll(rememberScrollState())
                .padding(32.dp),
        verticalArrangement = Arrangement.Top,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // Title row with settings gear
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Spacer(modifier = Modifier.size(48.dp)) // balance the gear icon
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                VisioLogo(size = 120.dp)
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = Strings.t("app.title", lang),
                    style = MaterialTheme.typography.headlineLarge,
                    color = MaterialTheme.colorScheme.onBackground,
                    fontWeight = FontWeight.Bold,
                )
            }
            IconButton(
                onClick = onSettings,
                modifier = Modifier.size(48.dp),
            ) {
                Icon(
                    painter = painterResource(R.drawable.ri_settings_3_line),
                    contentDescription = Strings.t("settings", lang),
                    tint = if (isDark) VisioColors.White else VisioColors.Greyscale400,
                    modifier = Modifier.size(24.dp),
                )
            }
        }

        Spacer(modifier = Modifier.height(8.dp))

        Text(
            text = Strings.t("home.subtitle", lang),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onBackground.copy(alpha = 0.7f),
        )

        Spacer(modifier = Modifier.height(16.dp))

        // Connect / Logout section
        if (VisioManager.isAuthenticated) {
            AuthenticatedCard(
                displayName = VisioManager.authenticatedDisplayName,
                email = VisioManager.authenticatedEmail,
                isDark = isDark,
                lang = lang,
                onLogout = { VisioManager.logout() },
            )
        } else {
            Button(
                onClick = {
                    if (meetInstances.size <= 1) {
                        val meetInstance = meetInstances.firstOrNull() ?: return@Button
                        VisioManager.authManager.launchOidcFlow(oidcLauncher, context, meetInstance)
                    } else {
                        customServer = ""
                        showServerPicker = true
                    }
                },
                modifier = Modifier.fillMaxWidth(),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = VisioColors.Primary500,
                        contentColor = VisioColors.White,
                    ),
                shape = RoundedCornerShape(12.dp),
            ) {
                Icon(
                    painter = painterResource(R.drawable.ri_account_circle_line),
                    contentDescription = null,
                    modifier = Modifier.size(20.dp),
                )
                Text(
                    Strings.t("home.connect", lang),
                    fontSize = 16.sp,
                    modifier = Modifier.padding(start = 8.dp, top = 4.dp, bottom = 4.dp),
                )
            }
        }

        if (showServerPicker) {
            ServerPickerDialog(
                instances = meetInstances,
                customServer = customServer,
                onCustomServerChange = { customServer = it },
                lang = lang,
                onSelect = { instance ->
                    showServerPicker = false
                    VisioManager.authManager.launchOidcFlow(oidcLauncher, context, instance)
                },
                onDismiss = { showServerPicker = false },
            )
        }

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = Strings.t("home.meetUrl", lang),
            style = MaterialTheme.typography.bodySmall,
            color = if (isDark) VisioColors.Greyscale400 else VisioColors.LightTextSecondary,
            modifier = Modifier.fillMaxWidth().padding(bottom = 4.dp),
        )
        TextField(
            value = roomUrl,
            onValueChange = { roomUrl = it },
            placeholder = {
                Text(
                    "abc-defg-hij",
                    color = if (isDark) VisioColors.Greyscale400 else VisioColors.LightTextSecondary,
                )
            },
            singleLine = true,
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Uri,
                autoCorrectEnabled = false,
                capitalization = KeyboardCapitalization.None,
            ),
            modifier = Modifier.fillMaxWidth(),
            colors =
                TextFieldDefaults.colors(
                    focusedContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                    unfocusedContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                    cursorColor = VisioColors.Primary500,
                    focusedTextColor = MaterialTheme.colorScheme.onSurface,
                    unfocusedTextColor = MaterialTheme.colorScheme.onSurface,
                    focusedLabelColor = VisioColors.Primary500,
                    unfocusedLabelColor = if (isDark) VisioColors.Greyscale400 else VisioColors.LightTextSecondary,
                    focusedIndicatorColor = Color.Transparent,
                    unfocusedIndicatorColor = Color.Transparent,
                ),
            shape = RoundedCornerShape(12.dp),
        )

        when (roomStatus) {
            "checking" ->
                Text(
                    Strings.t("home.room.checking", lang),
                    style = MaterialTheme.typography.bodySmall,
                    color = VisioColors.Greyscale400,
                    modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
                    textAlign = androidx.compose.ui.text.style.TextAlign.End,
                )
            "valid" ->
                Text(
                    Strings.t("home.room.valid", lang),
                    style = MaterialTheme.typography.bodySmall,
                    color = Color(0xFF18753C),
                    modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
                    textAlign = androidx.compose.ui.text.style.TextAlign.End,
                )
            "not_found" ->
                Text(
                    Strings.t("home.room.notFound", lang),
                    style = MaterialTheme.typography.bodySmall,
                    color = Color(0xFFE1000F),
                    modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
                    textAlign = androidx.compose.ui.text.style.TextAlign.End,
                )
        }

        Spacer(modifier = Modifier.height(16.dp))

        TextField(
            value = username,
            onValueChange = { username = it },
            label = {
                Text(
                    Strings.t("home.displayName", lang),
                    color = if (isDark) VisioColors.Greyscale400 else VisioColors.LightTextSecondary,
                )
            },
            placeholder = {
                Text(
                    Strings.t("home.displayName.placeholder", lang),
                    color = if (isDark) VisioColors.Greyscale400 else VisioColors.LightTextSecondary,
                )
            },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
            colors =
                TextFieldDefaults.colors(
                    focusedContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                    unfocusedContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                    cursorColor = VisioColors.Primary500,
                    focusedTextColor = MaterialTheme.colorScheme.onSurface,
                    unfocusedTextColor = MaterialTheme.colorScheme.onSurface,
                    focusedLabelColor = VisioColors.Primary500,
                    unfocusedLabelColor = if (isDark) VisioColors.Greyscale400 else VisioColors.LightTextSecondary,
                    focusedIndicatorColor = Color.Transparent,
                    unfocusedIndicatorColor = Color.Transparent,
                ),
            shape = RoundedCornerShape(12.dp),
        )

        Spacer(modifier = Modifier.height(24.dp))

        Button(
            onClick = { onJoin(resolveRoomUrl(roomUrl), username.trim()) },
            enabled = roomStatus == "valid",
            modifier = Modifier.fillMaxWidth(),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = VisioColors.Primary500,
                    contentColor = VisioColors.White,
                    disabledContainerColor = VisioColors.PrimaryDark300,
                    disabledContentColor = VisioColors.Greyscale400,
                ),
            shape = RoundedCornerShape(12.dp),
        ) {
            Text(
                Strings.t("home.join", lang),
                fontSize = 16.sp,
                modifier = Modifier.padding(vertical = 4.dp),
            )
        }

        if (VisioManager.isAuthenticated) {
            Spacer(modifier = Modifier.height(12.dp))
            OutlinedButton(
                onClick = { showCreateRoom = true },
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(12.dp),
            ) {
                Text(
                    Strings.t("home.createRoom", lang),
                    fontSize = 16.sp,
                    modifier = Modifier.padding(vertical = 4.dp),
                )
            }
        }
    }

    if (showCreateRoom) {
        CreateRoomDialog(
            meetInstance = VisioManager.authenticatedMeetInstance,
            lang = lang,
            onCreated = { roomUrl ->
                showCreateRoom = false
                onJoin(roomUrl, username)
            },
            onDismiss = { showCreateRoom = false },
        )
    }
}

@Composable
private fun AuthenticatedCard(
    displayName: String,
    email: String,
    isDark: Boolean,
    lang: String,
    onLogout: () -> Unit,
) {
    val initials = displayName
        .split(" ")
        .filter { it.isNotEmpty() }
        .take(2)
        .joinToString("") { it.first().uppercase() }
        .ifEmpty { email.firstOrNull()?.uppercase()?.toString() ?: "?" }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(
                color = if (isDark) VisioColors.PrimaryDark100 else VisioColors.LightSurfaceVariant,
                shape = RoundedCornerShape(16.dp),
            )
            .padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        // Avatar circle with initials
        Box(
            modifier = Modifier
                .size(44.dp)
                .background(
                    color = VisioColors.Primary500,
                    shape = RoundedCornerShape(22.dp),
                ),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = initials,
                color = VisioColors.White,
                fontWeight = FontWeight.Bold,
                fontSize = 16.sp,
            )
        }
        // Name and email
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = displayName.ifEmpty { email },
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
            )
            if (email.isNotEmpty() && displayName.isNotEmpty()) {
                Text(
                    text = email,
                    style = MaterialTheme.typography.bodySmall,
                    color = if (isDark) VisioColors.Greyscale400 else VisioColors.LightTextSecondary,
                    maxLines = 1,
                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                )
            }
        }
        // Logout button
        IconButton(
            onClick = onLogout,
            modifier = Modifier.size(36.dp),
        ) {
            Icon(
                painter = painterResource(R.drawable.ri_logout_box_r_line),
                contentDescription = Strings.t("home.logout", lang),
                tint = if (isDark) VisioColors.Greyscale400 else VisioColors.LightTextSecondary,
                modifier = Modifier.size(20.dp),
            )
        }
    }
}

@Composable
private fun CreateRoomDialog(
    meetInstance: String,
    lang: String,
    onCreated: (roomUrl: String) -> Unit,
    onDismiss: () -> Unit,
) {
    if (meetInstance.isEmpty()) return
    var accessLevel by remember { mutableStateOf("public") }
    var creating by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var createdUrl by remember { mutableStateOf<String?>(null) }
    val coroutineScope = rememberCoroutineScope()
    val context = LocalContext.current
    val clipboardManager = LocalClipboardManager.current

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(Strings.t("home.createRoom", lang)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                if (createdUrl == null) {
                    Text(
                        text = Strings.t("home.createRoom.access", lang),
                        style = MaterialTheme.typography.labelMedium,
                    )

                    Row(verticalAlignment = Alignment.CenterVertically) {
                        RadioButton(
                            selected = accessLevel == "public",
                            onClick = { accessLevel = "public" },
                        )
                        Column(modifier = Modifier.padding(start = 4.dp)) {
                            Text(Strings.t("home.createRoom.public", lang), style = MaterialTheme.typography.bodyMedium)
                            Text(Strings.t("home.createRoom.publicDesc", lang), style = MaterialTheme.typography.bodySmall)
                        }
                    }

                    Row(verticalAlignment = Alignment.CenterVertically) {
                        RadioButton(
                            selected = accessLevel == "trusted",
                            onClick = { accessLevel = "trusted" },
                        )
                        Column(modifier = Modifier.padding(start = 4.dp)) {
                            Text(Strings.t("home.createRoom.trusted", lang), style = MaterialTheme.typography.bodyMedium)
                            Text(Strings.t("home.createRoom.trustedDesc", lang), style = MaterialTheme.typography.bodySmall)
                        }
                    }

                    if (error != null) {
                        Text(
                            text = error!!,
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                } else {
                    val deepLink = "visio://${createdUrl!!.removePrefix("https://")}"

                    Text(
                        text = Strings.t("settings.incall.roomInfo", lang),
                        style = MaterialTheme.typography.labelMedium,
                    )

                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Default.Public, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(Strings.t("settings.incall.roomLink", lang), style = MaterialTheme.typography.labelSmall, modifier = Modifier.weight(1f))
                        IconButton(onClick = { clipboardManager.setText(AnnotatedString(createdUrl!!)) }, modifier = Modifier.size(32.dp)) {
                            Icon(Icons.Default.ContentCopy, contentDescription = Strings.t("settings.incall.copied", lang), modifier = Modifier.size(16.dp))
                        }
                        IconButton(onClick = {
                            val sendIntent = Intent().apply {
                                action = Intent.ACTION_SEND
                                putExtra(Intent.EXTRA_TEXT, createdUrl)
                                type = "text/plain"
                            }
                            context.startActivity(Intent.createChooser(sendIntent, null))
                        }, modifier = Modifier.size(32.dp)) {
                            Icon(Icons.Default.Share, contentDescription = Strings.t("settings.incall.share", lang), modifier = Modifier.size(16.dp))
                        }
                    }
                    OutlinedTextField(
                        value = createdUrl!!,
                        onValueChange = {},
                        readOnly = true,
                        singleLine = true,
                        textStyle = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    Spacer(Modifier.height(8.dp))

                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Default.Smartphone, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(Strings.t("settings.incall.deepLink", lang), style = MaterialTheme.typography.labelSmall, modifier = Modifier.weight(1f))
                        IconButton(onClick = { clipboardManager.setText(AnnotatedString(deepLink)) }, modifier = Modifier.size(32.dp)) {
                            Icon(Icons.Default.ContentCopy, contentDescription = Strings.t("settings.incall.copied", lang), modifier = Modifier.size(16.dp))
                        }
                        IconButton(onClick = {
                            val sendIntent = Intent().apply {
                                action = Intent.ACTION_SEND
                                putExtra(Intent.EXTRA_TEXT, deepLink)
                                type = "text/plain"
                            }
                            context.startActivity(Intent.createChooser(sendIntent, null))
                        }, modifier = Modifier.size(32.dp)) {
                            Icon(Icons.Default.Share, contentDescription = Strings.t("settings.incall.share", lang), modifier = Modifier.size(16.dp))
                        }
                    }
                    OutlinedTextField(
                        value = deepLink,
                        onValueChange = {},
                        readOnly = true,
                        singleLine = true,
                        textStyle = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        },
        confirmButton = {
            if (createdUrl == null) {
                Button(
                    onClick = {
                        creating = true
                        error = null
                        coroutineScope.launch(Dispatchers.IO) {
                            try {
                                val result = VisioManager.client.createRoom(
                                    "https://$meetInstance",
                                    "",
                                    accessLevel,
                                )
                                withContext(Dispatchers.Main) {
                                    createdUrl = "https://$meetInstance/${result.slug}"
                                    creating = false
                                }
                            } catch (e: Exception) {
                                withContext(Dispatchers.Main) {
                                    error = e.message ?: Strings.t("home.createRoom.error", lang)
                                    creating = false
                                }
                            }
                        }
                    },
                    enabled = !creating,
                ) {
                    Text(
                        if (creating) Strings.t("home.createRoom.creating", lang)
                        else Strings.t("home.createRoom.create", lang)
                    )
                }
            } else {
                Button(onClick = { onCreated(createdUrl!!) }) {
                    Text(Strings.t("home.join", lang))
                }
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text(Strings.t("settings.cancel", lang))
            }
        },
    )
}

@Composable
private fun ServerPickerDialog(
    instances: List<String>,
    customServer: String,
    onCustomServerChange: (String) -> Unit,
    lang: String,
    onSelect: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(Strings.t("home.serverPicker.title", lang)) },
        text = {
            Column {
                instances.forEach { instance ->
                    Text(
                        text = instance,
                        style = MaterialTheme.typography.bodyLarge,
                        color = VisioColors.Primary500,
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { onSelect(instance) }
                            .padding(vertical = 12.dp),
                    )
                }
                HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
                OutlinedTextField(
                    value = customServer,
                    onValueChange = onCustomServerChange,
                    label = { Text(Strings.t("home.serverPicker.custom", lang)) },
                    placeholder = { Text("meet.example.com") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Uri,
                        autoCorrectEnabled = false,
                        capitalization = KeyboardCapitalization.None,
                    ),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = { if (customServer.isNotBlank()) onSelect(customServer.trim()) },
                enabled = customServer.isNotBlank(),
            ) {
                Text(Strings.t("home.connect", lang))
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text(Strings.t("home.serverPicker.cancel", lang))
            }
        },
    )
}
