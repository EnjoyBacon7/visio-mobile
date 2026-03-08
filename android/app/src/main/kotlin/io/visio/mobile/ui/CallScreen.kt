package io.visio.mobile.ui

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import io.visio.mobile.R
import io.visio.mobile.ReactionData
import io.visio.mobile.VideoSurfaceView
import io.visio.mobile.VisioManager
import io.visio.mobile.ui.i18n.Strings
import io.visio.mobile.ui.theme.VisioColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.visio.AdaptiveMode
import uniffi.visio.ConnectionState
import uniffi.visio.ParticipantInfo
import uniffi.visio.WaitingParticipant
import kotlin.math.absoluteValue

private const val TAG = "CallScreen"

private val REACTION_EMOJIS =
    listOf(
        "thumbs-up" to "\uD83D\uDC4D",
        "thumbs-down" to "\uD83D\uDC4E",
        "clapping-hands" to "\uD83D\uDC4F",
        "red-heart" to "\u2764\uFE0F",
        "face-with-tears-of-joy" to "\uD83D\uDE02",
        "face-with-open-mouth" to "\uD83D\uDE2E",
        "party-popper" to "\uD83C\uDF89",
        "folded-hands" to "\uD83D\uDE4F",
    )

fun Context.findActivity(): Activity? {
    var ctx = this
    while (ctx is android.content.ContextWrapper) {
        if (ctx is Activity) return ctx
        ctx = ctx.baseContext
    }
    return null
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CallScreen(
    roomUrl: String,
    username: String,
    onNavigateToChat: () -> Unit,
    onHangUp: () -> Unit,
) {
    val connectionState by VisioManager.connectionState.collectAsState()
    val participants by VisioManager.participants.collectAsState()
    val activeSpeakers by VisioManager.activeSpeakers.collectAsState()
    val handRaisedMap by VisioManager.handRaisedMap.collectAsState()
    val unreadCount by VisioManager.unreadCount.collectAsState()
    val isHandRaised by VisioManager.isHandRaised.collectAsState()
    val lobbyNotification by VisioManager.lobbyNotification.collectAsState()
    val waitingParticipants by VisioManager.waitingParticipants.collectAsState()
    val adaptiveMode by VisioManager.adaptiveMode.collectAsState()

    val context = LocalContext.current
    val lang = VisioManager.currentLang
    var micEnabled by remember { mutableStateOf(false) }
    var cameraEnabled by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var showInCallSettings by remember { mutableStateOf(false) }
    var inCallSettingsTab by remember { mutableIntStateOf(0) }
    var showParticipantList by remember { mutableStateOf(false) }
    var focusedParticipantSid by remember { mutableStateOf<String?>(null) }
    var showReactionPicker by remember { mutableStateOf(false) }
    val reactions by VisioManager.reactions.collectAsState()

    var lastMode by remember { mutableStateOf(adaptiveMode) }

    LaunchedEffect(adaptiveMode) {
        if (adaptiveMode != lastMode) {
            lastMode = adaptiveMode
            // Sync local cameraEnabled with actual Rust state (FFI call off main thread)
            val camState = withContext(Dispatchers.IO) { VisioManager.client.isCameraEnabled() }
            cameraEnabled = camState
        }
    }

    val coroutineScope = rememberCoroutineScope()

    // Check if in PiP mode
    val isInPiP =
        context.findActivity()?.let {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) it.isInPictureInPictureMode else false
        } ?: false

    // Mic permission launcher
    val micPermissionLauncher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.RequestPermission(),
        ) { granted ->
            if (granted) {
                coroutineScope.launch(Dispatchers.IO) {
                    try {
                        VisioManager.client.setMicrophoneEnabled(true)
                        VisioManager.startAudioCapture()
                        micEnabled = true
                    } catch (e: Exception) {
                        Log.e(TAG, "Failed to enable microphone after permission grant", e)
                    }
                }
            }
        }

    // Camera permission launcher
    val cameraPermissionLauncher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.RequestPermission(),
        ) { granted ->
            if (granted) {
                coroutineScope.launch(Dispatchers.IO) {
                    try {
                        VisioManager.client.setCameraEnabled(true)
                        VisioManager.startCameraCapture()
                        cameraEnabled = true
                    } catch (e: Exception) {
                        Log.e(TAG, "Failed to enable camera after permission grant", e)
                    }
                }
            }
        }

    // Bluetooth permission launcher (needed for car kit detection on Android 12+)
    val bluetoothPermissionLauncher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.RequestPermission(),
        ) { granted ->
            if (granted) {
                Log.d(TAG, "BLUETOOTH_CONNECT permission granted")
            }
        }

    // Keep screen on while connected or reconnecting
    val keepScreenOn =
        connectionState is ConnectionState.Connected ||
            connectionState is ConnectionState.Reconnecting
    DisposableEffect(keepScreenOn) {
        val window = context.findActivity()?.window
        if (keepScreenOn) {
            window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } else {
            window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
        onDispose {
            window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }

    // Handle lobby denied — show toast and navigate back
    val lobbyDenied by VisioManager.lobbyDenied.collectAsState()
    LaunchedEffect(lobbyDenied) {
        if (lobbyDenied) {
            Toast.makeText(context, Strings.t("lobby.denied", lang), Toast.LENGTH_LONG).show()
            VisioManager.lobbyDenied.value = false
            VisioManager.disconnect()
            onHangUp()
        }
    }

    // WaitingForHost: show waiting screen instead of call UI
    if (connectionState is ConnectionState.WaitingForHost) {
        WaitingScreen(
            onCancel = {
                VisioManager.cancelLobby()
                VisioManager.disconnect()
                onHangUp()
            },
        )
        return
    }

    // Connect on first composition
    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) {
            val state = VisioManager.connectionState.value
            if (state is ConnectionState.Connected || state is ConnectionState.Connecting) {
                micEnabled = VisioManager.client.isMicrophoneEnabled()
                cameraEnabled = VisioManager.client.isCameraEnabled()
                return@withContext
            }

            val settings =
                try {
                    VisioManager.client.getSettings()
                } catch (e: Exception) {
                    errorMessage = "Failed to load settings: ${e.message}"
                    return@withContext
                }

            val user = username.ifBlank { null }
            try {
                VisioManager.client.connect(roomUrl, user)
            } catch (e: Exception) {
                errorMessage = "Connection failed: ${e.message}"
                return@withContext
            }

            try {
                VisioManager.startAudioPlayout()
            } catch (e: Exception) {
                errorMessage = "Audio playout failed: ${e.message}"
                return@withContext
            }

            // Apply mic-on-join setting (only if permission already granted)
            if (settings.micEnabledOnJoin) {
                val hasMicPerm =
                    ContextCompat.checkSelfPermission(
                        context, Manifest.permission.RECORD_AUDIO,
                    ) == PackageManager.PERMISSION_GRANTED
                if (hasMicPerm) {
                    try {
                        VisioManager.client.setMicrophoneEnabled(true)
                        VisioManager.startAudioCapture()
                    } catch (e: Exception) {
                        Log.e(TAG, "Failed to enable microphone on join", e)
                    }
                }
            }
            micEnabled = VisioManager.client.isMicrophoneEnabled()

            // Apply camera-on-join setting (only if permission already granted)
            if (settings.cameraEnabledOnJoin) {
                val hasCamPerm =
                    ContextCompat.checkSelfPermission(
                        context, Manifest.permission.CAMERA,
                    ) == PackageManager.PERMISSION_GRANTED
                if (hasCamPerm) {
                    try {
                        VisioManager.client.setCameraEnabled(true)
                        VisioManager.startCameraCapture()
                        VisioManager.refreshParticipantsPublic()
                    } catch (e: Exception) {
                        Log.e(TAG, "Failed to enable camera on join", e)
                    }
                }
            }
            cameraEnabled = VisioManager.client.isCameraEnabled()
        }
    }

    // Request BLUETOOTH_CONNECT permission on Android 12+ for car kit detection
    LaunchedEffect(Unit) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val hasBtPerm = ContextCompat.checkSelfPermission(
                context, Manifest.permission.BLUETOOTH_CONNECT,
            ) == PackageManager.PERMISSION_GRANTED
            if (!hasBtPerm) {
                bluetoothPermissionLauncher.launch(Manifest.permission.BLUETOOTH_CONNECT)
            }
        }
    }

    // Notify backend when navigating to chat
    val onChatOpen = {
        coroutineScope.launch(Dispatchers.IO) {
            try {
                VisioManager.client.setChatOpen(true)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to notify backend that chat is open", e)
            }
        }
        onNavigateToChat()
    }

    // PiP mode: show only active speaker, no controls
    if (isInPiP) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(VisioColors.PrimaryDark50),
            contentAlignment = Alignment.Center,
        ) {
            val activeSpeakerSid = activeSpeakers.firstOrNull()
            val speaker = participants.find { it.sid == activeSpeakerSid } ?: participants.firstOrNull()
            if (speaker != null) {
                ParticipantTile(
                    participant = speaker,
                    isActiveSpeaker = false,
                    handRaisePosition = 0,
                    onClick = {},
                )
            }
        }
        return
    }

    // Participant list bottom sheet
    if (showParticipantList) {
        ParticipantListSheet(
            participants = participants,
            localDisplayName = username,
            localMicEnabled = micEnabled,
            localCameraEnabled = cameraEnabled,
            localIsHandRaised = isHandRaised,
            handRaisedMap = handRaisedMap,
            lang = lang,
            onDismiss = { showParticipantList = false },
        )
    }

    // In-call settings bottom sheet (replaces audio device sheet)
    if (showInCallSettings) {
        InCallSettingsSheet(
            roomUrl = roomUrl,
            initialTab = inCallSettingsTab,
            onDismiss = { showInCallSettings = false },
            onSelectAudioInput = { device -> VisioManager.setAudioInputDevice(device) },
            onSelectAudioOutput = { device -> VisioManager.setAudioOutputDevice(device) },
            onSwitchCamera = { useFront ->
                VisioManager.switchCamera(useFront)
            },
            isFrontCamera = VisioManager.isFrontCamera(),
        )
    }

    // Main call layout
    val callBackground = if (adaptiveMode == AdaptiveMode.OFFICE) VisioColors.PrimaryDark50 else Color.Black
    Box(
        modifier =
            Modifier
                .fillMaxSize()
                .background(callBackground),
    ) {
        Column(modifier = Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding()) {
            // Connection state banner
            ConnectionStateBanner(connectionState, errorMessage)

            // Video grid area with reaction overlay
            Box(
                modifier =
                    Modifier
                        .weight(1f)
                        .fillMaxWidth()
                        .padding(8.dp),
            ) {
                when (adaptiveMode) {
                    AdaptiveMode.CAR -> {
                        // Car mode: audio-only view with active speaker name
                        val activeSpeakerSid = activeSpeakers.firstOrNull()
                        val speaker = participants.find { it.sid == activeSpeakerSid } ?: participants.firstOrNull()
                        val speakerName = speaker?.name ?: speaker?.identity ?: ""

                        Box(
                            modifier = Modifier.fillMaxSize(),
                            contentAlignment = Alignment.Center,
                        ) {
                            Column(
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.Center,
                            ) {
                                Icon(
                                    painter = painterResource(R.drawable.ri_mic_line),
                                    contentDescription = null,
                                    tint = VisioColors.Primary500,
                                    modifier = Modifier.size(64.dp),
                                )
                                Spacer(modifier = Modifier.height(24.dp))
                                Text(
                                    text = speakerName,
                                    color = Color.White,
                                    fontSize = 32.sp,
                                    fontWeight = FontWeight.Bold,
                                    textAlign = TextAlign.Center,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.fillMaxWidth().padding(horizontal = 32.dp),
                                )
                                Spacer(modifier = Modifier.height(12.dp))
                                Text(
                                    text = Strings.t("adaptive.audioOnly", lang),
                                    color = Color.White.copy(alpha = 0.6f),
                                    fontSize = 16.sp,
                                )
                            }
                        }
                    }

                    AdaptiveMode.PEDESTRIAN -> {
                        // Pedestrian mode: single active speaker tile
                        val activeSpeakerSid = activeSpeakers.firstOrNull()
                        // Find if active speaker is a remote participant
                        // (participants[0] is local, so skip it when looking for remote speaker)
                        val remoteSpeaker = if (activeSpeakerSid != null) {
                            participants.drop(1).find { it.sid == activeSpeakerSid }
                        } else null

                        Box(
                            modifier = Modifier
                                .fillMaxSize()
                                .clip(RoundedCornerShape(8.dp)),
                        ) {
                            if (remoteSpeaker != null) {
                                // Show remote active speaker
                                ParticipantTile(
                                    participant = remoteSpeaker,
                                    isActiveSpeaker = true,
                                    handRaisePosition = handRaisedMap[remoteSpeaker.sid] ?: 0,
                                    onClick = {},
                                )
                            } else {
                                // No remote speaker talking — show first remote participant or local preview
                                val fallback = participants.firstOrNull()
                                if (fallback != null) {
                                    ParticipantTile(
                                        participant = fallback,
                                        isActiveSpeaker = activeSpeakers.contains(fallback.sid),
                                        handRaisePosition = handRaisedMap[fallback.sid] ?: 0,
                                        onClick = {},
                                    )
                                }
                            }
                        }
                    }

                    AdaptiveMode.OFFICE -> {
                        // Office mode: full grid (existing behavior)
                        val focusedP = focusedParticipantSid?.let { sid -> participants.find { it.sid == sid } }

                        if (focusedP != null) {
                            // Focus mode — full-screen focused participant
                            Box(
                                modifier =
                                    Modifier
                                        .fillMaxSize()
                                        .clip(RoundedCornerShape(8.dp)),
                            ) {
                                ParticipantTile(
                                    participant = focusedP,
                                    isActiveSpeaker = activeSpeakers.contains(focusedP.sid),
                                    handRaisePosition = handRaisedMap[focusedP.sid] ?: 0,
                                    onClick = { focusedParticipantSid = null },
                                )
                            }
                        } else {
                            // Grid mode — space-filling tiles
                            val count = participants.size
                            BoxWithConstraints(modifier = Modifier.fillMaxSize()) {
                                val isLandscape = maxWidth > maxHeight
                                val columnCount =
                                    when {
                                        count == 1 -> 1
                                        isLandscape -> minOf(count, 3)
                                        count <= 2 -> 1
                                        else -> 2
                                    }
                                val rowCount = (count + columnCount - 1) / columnCount
                                val tileHeight = (maxHeight - 8.dp * (rowCount - 1)) / rowCount

                                Column(
                                    verticalArrangement = Arrangement.spacedBy(8.dp),
                                    modifier = Modifier.fillMaxSize(),
                                ) {
                                    for (rowStart in 0 until count step columnCount) {
                                        val rowEnd = minOf(rowStart + columnCount, count)
                                        Row(
                                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                                            modifier =
                                                Modifier
                                                    .fillMaxWidth()
                                                    .height(tileHeight),
                                        ) {
                                            for (idx in rowStart until rowEnd) {
                                                val p = participants[idx]
                                                Box(
                                                    modifier =
                                                        Modifier
                                                            .weight(1f)
                                                            .fillMaxHeight()
                                                            .clip(RoundedCornerShape(8.dp)),
                                                ) {
                                                    ParticipantTile(
                                                        participant = p,
                                                        isActiveSpeaker = activeSpeakers.contains(p.sid),
                                                        handRaisePosition = handRaisedMap[p.sid] ?: 0,
                                                        onClick = { focusedParticipantSid = p.sid },
                                                    )
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Reaction overlay on top of video grid
                ReactionOverlay(reactions = reactions)

                // Persistent adaptive mode indicator (on top of everything)
                Row(
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .padding(8.dp)
                        .background(Color.Black.copy(alpha = 0.6f), RoundedCornerShape(12.dp))
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp)
                ) {
                    val (modeIcon, modeKey) = when (adaptiveMode) {
                        uniffi.visio.AdaptiveMode.OFFICE -> "🏢" to "adaptive.office"
                        uniffi.visio.AdaptiveMode.PEDESTRIAN -> "🚶" to "adaptive.pedestrian"
                        uniffi.visio.AdaptiveMode.CAR -> "🚗" to "adaptive.car"
                    }
                    Text(text = modeIcon, fontSize = 12.sp)
                    Text(
                        text = Strings.t(modeKey, lang),
                        color = Color.White,
                        fontSize = 11.sp
                    )
                }
            }

            Spacer(modifier = Modifier.height(8.dp))

            // Control bar
            ControlBar(
                micEnabled = micEnabled,
                cameraEnabled = cameraEnabled,
                isHandRaised = isHandRaised,
                unreadCount = unreadCount,
                participantCount = participants.size,
                showReactionPicker = showReactionPicker,
                adaptiveMode = adaptiveMode,
                lang = lang,
                onToggleMic = {
                    val newState = !micEnabled
                    if (newState) {
                        val hasPermission =
                            ContextCompat.checkSelfPermission(
                                context, Manifest.permission.RECORD_AUDIO,
                            ) == PackageManager.PERMISSION_GRANTED
                        if (hasPermission) {
                            coroutineScope.launch(Dispatchers.IO) {
                                try {
                                    VisioManager.client.setMicrophoneEnabled(true)
                                    VisioManager.startAudioCapture()
                                    micEnabled = true
                                } catch (e: Exception) {
                                    Log.e(TAG, "Failed to enable microphone", e)
                                }
                            }
                        } else {
                            micPermissionLauncher.launch(Manifest.permission.RECORD_AUDIO)
                        }
                    } else {
                        coroutineScope.launch(Dispatchers.IO) {
                            try {
                                VisioManager.stopAudioCapture()
                                VisioManager.client.setMicrophoneEnabled(false)
                                micEnabled = false
                            } catch (e: Exception) {
                                Log.e(TAG, "Failed to disable microphone", e)
                            }
                        }
                    }
                },
                onAudioPicker = {
                    inCallSettingsTab = 1
                    showInCallSettings = true
                },
                onToggleCamera = {
                    val newState = !cameraEnabled
                    if (newState) {
                        val hasPermission =
                            ContextCompat.checkSelfPermission(
                                context, Manifest.permission.CAMERA,
                            ) == PackageManager.PERMISSION_GRANTED
                        if (hasPermission) {
                            coroutineScope.launch(Dispatchers.IO) {
                                try {
                                    VisioManager.client.setCameraEnabled(true)
                                    VisioManager.startCameraCapture()
                                    cameraEnabled = true
                                    VisioManager.refreshParticipantsPublic()
                                } catch (e: Exception) {
                                    Log.e(TAG, "Failed to enable camera", e)
                                }
                            }
                        } else {
                            cameraPermissionLauncher.launch(Manifest.permission.CAMERA)
                        }
                    } else {
                        coroutineScope.launch(Dispatchers.IO) {
                            try {
                                VisioManager.stopCameraCapture()
                                VisioManager.client.setCameraEnabled(false)
                                cameraEnabled = false
                                VisioManager.refreshParticipantsPublic()
                            } catch (e: Exception) {
                                Log.e(TAG, "Failed to disable camera", e)
                            }
                        }
                    }
                },
                onToggleHandRaise = {
                    coroutineScope.launch(Dispatchers.IO) {
                        try {
                            if (isHandRaised) {
                                VisioManager.client.lowerHand()
                            } else {
                                VisioManager.client.raiseHand()
                            }
                        } catch (e: Exception) {
                            Log.e(TAG, "Failed to toggle hand raise", e)
                        }
                    }
                },
                onReaction = { emoji ->
                    VisioManager.sendReaction(emoji)
                    showReactionPicker = false
                },
                onToggleReactionPicker = { showReactionPicker = !showReactionPicker },
                onParticipants = { showParticipantList = true },
                onSettings = {
                    inCallSettingsTab = 0
                    showInCallSettings = true
                },
                onChat = onChatOpen,
                onHangUp = {
                    VisioManager.disconnect()
                    onHangUp()
                },
                onAdaptiveModeOverride = { mode ->
                    coroutineScope.launch(Dispatchers.IO) {
                        VisioManager.client.setAdaptiveModeOverride(mode)
                    }
                },
            )

            Spacer(modifier = Modifier.height(8.dp))
        }

        // Lobby: persistent banner when participants are waiting
        LobbyWaitingBanner(
            waitingParticipants = waitingParticipants,
            lang = lang,
            onAdmit = { participant ->
                VisioManager.admitParticipant(participant.id)
            },
            onView = {
                showParticipantList = true
            },
            modifier =
                Modifier
                    .align(Alignment.TopCenter)
                    .statusBarsPadding()
                    .padding(top = 8.dp, start = 16.dp, end = 16.dp),
        )
    }
}

@Composable
private fun LobbyWaitingBanner(
    waitingParticipants: List<WaitingParticipant>,
    lang: String,
    onAdmit: (WaitingParticipant) -> Unit,
    onView: () -> Unit,
    modifier: Modifier = Modifier,
) {
    AnimatedVisibility(
        visible = waitingParticipants.isNotEmpty(),
        enter = slideInVertically { -it } + fadeIn(),
        exit = slideOutVertically { -it } + fadeOut(),
        modifier = modifier,
    ) {
        if (waitingParticipants.isNotEmpty()) {
            val first = waitingParticipants.first()
            val message =
                if (waitingParticipants.size == 1) {
                    Strings.t("lobby.joinRequest", lang).replace("{{name}}", first.username)
                } else {
                    Strings.t("lobby.joinRequest", lang).replace("{{name}}", first.username) +
                        " (+${waitingParticipants.size - 1})"
                }
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .shadow(8.dp, RoundedCornerShape(12.dp))
                        .background(VisioColors.PrimaryDark75, RoundedCornerShape(12.dp))
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    text = message,
                    color = VisioColors.White,
                    fontSize = 14.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                Button(
                    onClick = { onAdmit(first) },
                    colors =
                        ButtonDefaults.buttonColors(
                            containerColor = VisioColors.Primary500,
                        ),
                    shape = RoundedCornerShape(8.dp),
                    modifier = Modifier.height(32.dp),
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 0.dp),
                ) {
                    Text(
                        text = Strings.t("lobby.admit", lang),
                        fontSize = 12.sp,
                    )
                }
                OutlinedButton(
                    onClick = onView,
                    shape = RoundedCornerShape(8.dp),
                    modifier = Modifier.height(32.dp),
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 0.dp),
                ) {
                    Text(
                        text = Strings.t("lobby.view", lang),
                        fontSize = 12.sp,
                        color = VisioColors.White,
                    )
                }
            }
        }
    }
}

@Composable
private fun ControlBar(
    micEnabled: Boolean,
    cameraEnabled: Boolean,
    isHandRaised: Boolean,
    unreadCount: Int,
    participantCount: Int,
    showReactionPicker: Boolean,
    adaptiveMode: AdaptiveMode,
    lang: String,
    onToggleMic: () -> Unit,
    onAudioPicker: () -> Unit,
    onToggleCamera: () -> Unit,
    onToggleHandRaise: () -> Unit,
    onReaction: (String) -> Unit,
    onToggleReactionPicker: () -> Unit,
    onParticipants: () -> Unit,
    onSettings: () -> Unit,
    onChat: () -> Unit,
    onHangUp: () -> Unit,
    onAdaptiveModeOverride: (AdaptiveMode?) -> Unit,
) {
    var showOverflow by remember { mutableStateOf(false) }
    var adaptiveModeOverride by remember { mutableStateOf<AdaptiveMode?>(null) }

    Column(
        modifier = Modifier.fillMaxWidth(),
    ) {
        // Reaction picker (slides above control bar)
        if (showReactionPicker) {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp)
                        .background(Color(0xCC000000), RoundedCornerShape(12.dp))
                        .padding(horizontal = 8.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.SpaceEvenly,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                REACTION_EMOJIS.forEach { (id, emoji) ->
                    Text(
                        text = emoji,
                        fontSize = 28.sp,
                        modifier =
                            Modifier
                                .clickable { onReaction(id) }
                                .padding(4.dp),
                    )
                }
            }
        }

        // Overflow menu (slides above control bar)
        if (showOverflow) {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp)
                        .background(Color(0xCC000000), RoundedCornerShape(12.dp))
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.SpaceEvenly,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // Hand raise
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier =
                        Modifier.clickable {
                            onToggleHandRaise()
                            showOverflow = false
                        }.padding(horizontal = 8.dp),
                ) {
                    IconButton(
                        onClick = {
                            onToggleHandRaise()
                            showOverflow = false
                        },
                        modifier =
                            Modifier
                                .size(38.dp)
                                .background(
                                    if (isHandRaised) VisioColors.HandRaise else VisioColors.PrimaryDark100,
                                    RoundedCornerShape(8.dp),
                                ),
                    ) {
                        Icon(
                            painter = painterResource(R.drawable.ri_hand),
                            contentDescription =
                                if (isHandRaised) {
                                    Strings.t(
                                        "control.lowerHand",
                                        lang,
                                    )
                                } else {
                                    Strings.t("control.raiseHand", lang)
                                },
                            tint = if (isHandRaised) Color.Black else VisioColors.White,
                            modifier = Modifier.size(20.dp),
                        )
                    }
                    Text(
                        text = if (isHandRaised) Strings.t("control.lowerHand", lang) else Strings.t("control.raiseHand", lang),
                        color = VisioColors.White,
                        fontSize = 10.sp,
                        maxLines = 1,
                    )
                }

                // Reaction
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier =
                        Modifier.clickable {
                            showOverflow = false
                            onToggleReactionPicker()
                        }.padding(horizontal = 8.dp),
                ) {
                    IconButton(
                        onClick = {
                            showOverflow = false
                            onToggleReactionPicker()
                        },
                        modifier =
                            Modifier
                                .size(38.dp)
                                .background(VisioColors.PrimaryDark100, RoundedCornerShape(8.dp)),
                    ) {
                        Icon(
                            painter = painterResource(R.drawable.ri_emotion_line),
                            contentDescription = "Reaction",
                            tint = VisioColors.White,
                            modifier = Modifier.size(20.dp),
                        )
                    }
                    Text(
                        text = "Reaction",
                        color = VisioColors.White,
                        fontSize = 10.sp,
                        maxLines = 1,
                    )
                }

                // Settings
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier =
                        Modifier.clickable {
                            showOverflow = false
                            onSettings()
                        }.padding(horizontal = 8.dp),
                ) {
                    IconButton(
                        onClick = {
                            showOverflow = false
                            onSettings()
                        },
                        modifier =
                            Modifier
                                .size(38.dp)
                                .background(VisioColors.PrimaryDark100, RoundedCornerShape(8.dp)),
                    ) {
                        Icon(
                            painter = painterResource(R.drawable.ri_settings_3_line),
                            contentDescription = Strings.t("settings.incall", lang),
                            tint = VisioColors.White,
                            modifier = Modifier.size(20.dp),
                        )
                    }
                    Text(
                        text = Strings.t("settings.incall", lang),
                        color = VisioColors.White,
                        fontSize = 10.sp,
                        maxLines = 1,
                    )
                }
            }

            // Adaptive mode override
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp)
                    .background(Color(0xCC000000), RoundedCornerShape(12.dp))
                    .padding(horizontal = 12.dp, vertical = 8.dp),
            ) {
                Text(
                    text = Strings.t("adaptive.override", lang),
                    color = VisioColors.White,
                    fontSize = 11.sp,
                    fontWeight = FontWeight.Medium,
                    modifier = Modifier.padding(bottom = 6.dp),
                )
                Row(
                    horizontalArrangement = Arrangement.SpaceEvenly,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    val modeOptions = listOf<Pair<AdaptiveMode?, String>>(
                        null to Strings.t("adaptive.auto", lang),
                        AdaptiveMode.OFFICE to Strings.t("adaptive.office", lang),
                        AdaptiveMode.PEDESTRIAN to Strings.t("adaptive.pedestrian", lang),
                        AdaptiveMode.CAR to Strings.t("adaptive.car", lang),
                    )
                    modeOptions.forEach { (mode, label) ->
                        val isSelected = mode == adaptiveModeOverride
                        Text(
                            text = label,
                            color = if (isSelected) Color.Black else VisioColors.White,
                            fontSize = 11.sp,
                            fontWeight = if (isSelected) FontWeight.Bold else FontWeight.Normal,
                            modifier = Modifier
                                .background(
                                    if (isSelected) VisioColors.Primary500 else VisioColors.PrimaryDark100,
                                    RoundedCornerShape(16.dp),
                                )
                                .clickable {
                                    adaptiveModeOverride = mode
                                    onAdaptiveModeOverride(mode)
                                }
                                .padding(horizontal = 10.dp, vertical = 6.dp),
                            maxLines = 1,
                        )
                    }
                }
            }
        }

        // Main control bar — button sizes adapt to mode
        val isLargeButtons = adaptiveMode != AdaptiveMode.OFFICE
        val btnSize = if (isLargeButtons) 96.dp else 38.dp
        val iconSize = if (isLargeButtons) 48.dp else 20.dp
        val cornerRadius = if (isLargeButtons) 16.dp else 8.dp

        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp)
                    .background(VisioColors.PrimaryDark75, RoundedCornerShape(16.dp))
                    .padding(horizontal = 6.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.SpaceEvenly,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // Mic group: toggle + audio picker chevron
            Row(
                modifier =
                    Modifier
                        .background(
                            if (micEnabled) VisioColors.PrimaryDark100 else VisioColors.Error200,
                            RoundedCornerShape(cornerRadius),
                        ),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(
                    onClick = onToggleMic,
                    modifier = Modifier.size(btnSize),
                ) {
                    Icon(
                        painter =
                            painterResource(
                                if (micEnabled) R.drawable.ri_mic_line else R.drawable.ri_mic_off_line,
                            ),
                        contentDescription = if (micEnabled) Strings.t("control.mute", lang) else Strings.t("control.unmute", lang),
                        tint = VisioColors.White,
                        modifier = Modifier.size(iconSize),
                    )
                }
                // Audio device picker chevron — visible in all modes
                val chevronHeight = if (isLargeButtons) 96.dp else 38.dp
                val chevronWidth = if (isLargeButtons) 40.dp else 22.dp
                val chevronIconSize = if (isLargeButtons) 24.dp else 14.dp
                IconButton(
                    onClick = onAudioPicker,
                    modifier = Modifier.size(chevronWidth, chevronHeight),
                ) {
                    Icon(
                        painter = painterResource(R.drawable.ri_arrow_up_s_line),
                        contentDescription = Strings.t("control.audioDevices", lang),
                        tint = VisioColors.White,
                        modifier = Modifier.size(chevronIconSize),
                    )
                }
            }

            // Camera toggle — visible in OFFICE and PEDESTRIAN only
            if (adaptiveMode != AdaptiveMode.CAR) {
                IconButton(
                    onClick = onToggleCamera,
                    modifier =
                        Modifier
                            .size(btnSize)
                            .background(
                                if (cameraEnabled) VisioColors.PrimaryDark100 else VisioColors.Error200,
                                RoundedCornerShape(cornerRadius),
                            ),
                ) {
                    Icon(
                        painter =
                            painterResource(
                                if (cameraEnabled) R.drawable.ri_video_on_line else R.drawable.ri_video_off_line,
                            ),
                        contentDescription = if (cameraEnabled) Strings.t("control.camOff", lang) else Strings.t("control.camOn", lang),
                        tint = VisioColors.White,
                        modifier = Modifier.size(iconSize),
                    )
                }
            }

            // Participants with count badge — OFFICE only
            if (adaptiveMode == AdaptiveMode.OFFICE) {
                IconButton(
                    onClick = onParticipants,
                    modifier =
                        Modifier
                            .size(btnSize)
                            .background(VisioColors.PrimaryDark100, RoundedCornerShape(cornerRadius)),
                ) {
                    BadgedBox(
                        badge = {
                            if (participantCount > 0) {
                                Badge(
                                    containerColor = VisioColors.Primary500,
                                    contentColor = VisioColors.White,
                                ) {
                                    Text(
                                        text = "$participantCount",
                                        fontSize = 10.sp,
                                    )
                                }
                            }
                        },
                    ) {
                        Icon(
                            painter = painterResource(R.drawable.ri_group_line),
                            contentDescription = Strings.t("participants.title", lang),
                            tint = VisioColors.White,
                            modifier = Modifier.size(iconSize),
                        )
                    }
                }
            }

            // Chat with unread badge — OFFICE only
            if (adaptiveMode == AdaptiveMode.OFFICE) {
                IconButton(
                    onClick = onChat,
                    modifier =
                        Modifier
                            .size(btnSize)
                            .background(VisioColors.PrimaryDark100, RoundedCornerShape(cornerRadius)),
                ) {
                    BadgedBox(
                        badge = {
                            if (unreadCount > 0) {
                                Badge(
                                    containerColor = VisioColors.Error500,
                                    contentColor = VisioColors.White,
                                ) {
                                    Text(
                                        text = "$unreadCount",
                                        fontSize = 10.sp,
                                    )
                                }
                            }
                        },
                    ) {
                        Icon(
                            painter = painterResource(R.drawable.ri_chat_1_line),
                            contentDescription = Strings.t("chat", lang),
                            tint = VisioColors.White,
                            modifier = Modifier.size(iconSize),
                        )
                    }
                }
            }

            // More (overflow) button — OFFICE only
            if (adaptiveMode == AdaptiveMode.OFFICE) {
                IconButton(
                    onClick = {
                        showOverflow = !showOverflow
                        if (showOverflow) {
                            // Close reaction picker when opening overflow
                        }
                    },
                    modifier =
                        Modifier
                            .size(btnSize)
                            .background(
                                if (showOverflow) VisioColors.Primary500 else VisioColors.PrimaryDark100,
                                RoundedCornerShape(cornerRadius),
                            ),
                ) {
                    Icon(
                        painter = painterResource(R.drawable.ri_more_2_fill),
                        contentDescription = "More",
                        tint = VisioColors.White,
                        modifier = Modifier.size(iconSize),
                    )
                }
            }

            // Hangup
            IconButton(
                onClick = onHangUp,
                modifier =
                    Modifier
                        .size(btnSize)
                        .background(VisioColors.Error500, RoundedCornerShape(cornerRadius)),
            ) {
                Icon(
                    painter = painterResource(R.drawable.ri_phone_fill),
                    contentDescription = Strings.t("control.leave", lang),
                    tint = VisioColors.White,
                    modifier = Modifier.size(iconSize),
                )
            }
        }
    }
}

@Composable
fun ParticipantTile(
    participant: ParticipantInfo,
    isActiveSpeaker: Boolean,
    handRaisePosition: Int,
    onClick: () -> Unit,
) {
    val lang = VisioManager.currentLang
    val name = participant.name ?: participant.identity
    val initials =
        name
            .split(" ")
            .mapNotNull { it.firstOrNull()?.uppercase() }
            .take(2)
            .joinToString("")
            .ifEmpty { "?" }

    // Deterministic hue from name
    val hue = name.fold(0) { acc, c -> acc + c.code }.absoluteValue % 360
    val avatarColor = Color.hsl(hue.toFloat(), 0.5f, 0.35f)

    val borderColor = if (isActiveSpeaker) VisioColors.Primary500 else Color.Transparent
    val borderMod =
        if (isActiveSpeaker) {
            Modifier
                .border(2.dp, borderColor, RoundedCornerShape(8.dp))
                .shadow(8.dp, RoundedCornerShape(8.dp), ambientColor = VisioColors.Primary500)
        } else {
            Modifier
        }

    Box(
        modifier =
            Modifier
                .fillMaxSize()
                .then(borderMod)
                .clip(RoundedCornerShape(8.dp))
                .background(VisioColors.PrimaryDark50)
                .clickable(onClick = onClick),
    ) {
        // Video surface or avatar fallback
        if (participant.hasVideo && participant.videoTrackSid != null) {
            val trackSid = participant.videoTrackSid!!
            AndroidView(
                factory = { ctx -> VideoSurfaceView(ctx, trackSid) },
                modifier = Modifier.fillMaxSize(),
            )
        } else {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                Box(
                    modifier =
                        Modifier
                            .size(64.dp)
                            .clip(CircleShape)
                            .background(avatarColor),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = initials,
                        color = VisioColors.White,
                        fontSize = 24.sp,
                        fontWeight = FontWeight.Bold,
                    )
                }
            }
        }

        // Metadata bar at bottom
        Row(
            modifier =
                Modifier
                    .align(Alignment.BottomStart)
                    .fillMaxWidth()
                    .background(Color(0x99000000))
                    .padding(horizontal = 8.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            // Mic muted indicator
            if (participant.isMuted) {
                Icon(
                    painter = painterResource(R.drawable.ri_mic_off_fill),
                    contentDescription = Strings.t("accessibility.muted", lang),
                    tint = VisioColors.Error500,
                    modifier = Modifier.size(14.dp),
                )
            }

            // Hand raise badge
            if (handRaisePosition > 0) {
                Row(
                    modifier =
                        Modifier
                            .background(VisioColors.HandRaise, RoundedCornerShape(10.dp))
                            .padding(horizontal = 6.dp, vertical = 1.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    Icon(
                        painter = painterResource(R.drawable.ri_hand),
                        contentDescription = null,
                        tint = Color.Black,
                        modifier = Modifier.size(12.dp),
                    )
                    Text(
                        text = "$handRaisePosition",
                        color = Color.Black,
                        fontSize = 11.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                }
            }

            // Name
            Text(
                text = name,
                color = VisioColors.White,
                fontSize = 12.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )

            // Connection quality bars
            ConnectionQualityBars(participant.connectionQuality.name)
        }
    }
}

@Composable
private fun ConnectionQualityBars(quality: String) {
    val bars =
        when (quality) {
            "Excellent" -> 3
            "Good" -> 2
            "Poor" -> 1
            else -> 0
        }
    Row(
        horizontalArrangement = Arrangement.spacedBy(1.dp),
        verticalAlignment = Alignment.Bottom,
    ) {
        for (i in 1..3) {
            Box(
                modifier =
                    Modifier
                        .width(3.dp)
                        .height((i * 4 + 2).dp)
                        .background(
                            if (i <= bars) Color.Green else VisioColors.Greyscale400,
                            RoundedCornerShape(1.dp),
                        ),
            )
        }
    }
}

@Composable
private fun ConnectionStateBanner(
    state: ConnectionState,
    errorMessage: String?,
) {
    val lang = VisioManager.currentLang
    when {
        errorMessage != null -> {
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .background(VisioColors.Error200)
                        .padding(12.dp),
            ) {
                Text(
                    text = "${Strings.t("call.error", lang)}: $errorMessage",
                    color = VisioColors.Error500,
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
        state is ConnectionState.Connecting -> {
            Row(
                modifier = Modifier.padding(12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(20.dp),
                    color = VisioColors.Primary500,
                )
                Text(
                    "${Strings.t("status.connecting", lang)}...",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onBackground,
                )
            }
        }
        state is ConnectionState.Reconnecting -> {
            Row(
                modifier = Modifier.padding(12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(20.dp),
                    color = VisioColors.Primary500,
                )
                Text(
                    "${Strings.t("status.reconnecting", lang)} (${state.attempt})...",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onBackground,
                )
            }
        }
        // Connected / Disconnected: no banner
    }
}

@Composable
private fun WaitingScreen(onCancel: () -> Unit) {
    val lang = VisioManager.currentLang

    Box(
        modifier =
            Modifier
                .fillMaxSize()
                .background(VisioColors.PrimaryDark50),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            CircularProgressIndicator(
                modifier = Modifier.size(48.dp),
                color = VisioColors.Primary500,
            )
            Spacer(modifier = Modifier.height(24.dp))
            Text(
                text = Strings.t("lobby.waiting", lang),
                style = MaterialTheme.typography.titleMedium,
                color = VisioColors.White,
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = Strings.t("lobby.waitingDesc", lang),
                style = MaterialTheme.typography.bodyMedium,
                color = VisioColors.Greyscale400,
            )
            Spacer(modifier = Modifier.height(24.dp))
            OutlinedButton(onClick = onCancel) {
                Text(Strings.t("lobby.cancel", lang))
            }
        }
    }
}

@Composable
private fun ReactionOverlay(reactions: List<ReactionData>) {
    val now = System.currentTimeMillis()
    val activeReactions = reactions.filter { now - it.timestamp < 3000L }

    // Periodically trigger recomposition to remove expired reactions
    var tick by remember { mutableStateOf(0L) }
    LaunchedEffect(reactions.size) {
        if (reactions.isNotEmpty()) {
            delay(3100L)
            tick = System.currentTimeMillis()
        }
    }

    Box(modifier = Modifier.fillMaxSize()) {
        activeReactions.forEach { reaction ->
            FloatingReaction(reaction = reaction, modifier = Modifier.align(Alignment.BottomStart))
        }
    }
}

@Composable
private fun FloatingReaction(
    reaction: ReactionData,
    modifier: Modifier = Modifier,
) {
    val emojiDisplay = REACTION_EMOJIS.find { it.first == reaction.emoji }?.second ?: reaction.emoji
    val density = LocalDensity.current
    val screenWidthDp = LocalConfiguration.current.screenWidthDp
    // Deterministic horizontal position based on reaction id (left 20% of screen)
    val xOffsetDp =
        remember(reaction.id) {
            ((reaction.id * 37 + 13) % (screenWidthDp * 20 / 100)).toInt()
        }

    val progress = remember { Animatable(0f) }

    LaunchedEffect(reaction.id) {
        progress.animateTo(
            targetValue = 1f,
            animationSpec = tween(durationMillis = 3000, easing = LinearEasing),
        )
    }

    val yOffset = with(density) { (-300.dp * progress.value).roundToPx() }
    val alphaValue =
        if (progress.value > 0.7f) {
            1f - ((progress.value - 0.7f) / 0.3f)
        } else {
            1f
        }

    Column(
        modifier =
            modifier
                .offset { IntOffset(with(density) { xOffsetDp.dp.roundToPx() }, yOffset) }
                .alpha(alphaValue),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = emojiDisplay,
            fontSize = 32.sp,
        )
        Text(
            text = reaction.participantName,
            color = VisioColors.White,
            fontSize = 10.sp,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
            modifier =
                Modifier
                    .background(Color(0x99000000), RoundedCornerShape(4.dp))
                    .padding(horizontal = 4.dp, vertical = 1.dp),
        )
    }
}
