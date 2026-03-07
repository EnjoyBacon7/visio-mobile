package io.visio.mobile

import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothClass
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log
import uniffi.visio.NetworkType

class ContextDetector(private val context: Context) {

    private val connectivityManager =
        context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private val sensorManager =
        context.getSystemService(Context.SENSOR_SERVICE) as SensorManager
    private val audioManager =
        context.getSystemService(Context.AUDIO_SERVICE) as AudioManager

    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var accelerometerListener: SensorEventListener? = null
    private var audioDeviceCallback: AudioDeviceCallback? = null

    private var lastReportedMotion = false
    private var lastSignificantMotionMs = 0L  // last time we saw a significant accel event
    private val MOTION_THRESHOLD = 1.5f       // m/s² deviation from gravity to count as motion
    private val MOTION_COOLDOWN_MS = 10000L   // stay in pedestrian 10s after last motion event

    companion object {
        private const val TAG = "ContextDetector"
    }

    fun start() {
        Log.i(TAG, "Starting context detection (network, motion, bluetooth)")
        startNetworkMonitoring()
        startMotionDetection()
        startBluetoothMonitoring()
        reportCurrentNetworkType()
        reportBluetoothCarKit()
    }

    fun stop() {
        networkCallback?.let { connectivityManager.unregisterNetworkCallback(it) }
        accelerometerListener?.let { sensorManager.unregisterListener(it) }
        audioDeviceCallback?.let { audioManager.unregisterAudioDeviceCallback(it) }
        networkCallback = null
        accelerometerListener = null
        audioDeviceCallback = null
    }

    private fun startNetworkMonitoring() {
        val request = NetworkRequest.Builder().build()
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                val type = when {
                    caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> NetworkType.WIFI
                    caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> NetworkType.CELLULAR
                    else -> NetworkType.UNKNOWN
                }
                Log.d(TAG, "Network type: $type")
                try { VisioManager.client.reportNetworkType(type) } catch (_: Exception) {}
            }
            override fun onLost(network: Network) {
                Log.d(TAG, "Network lost")
                try { VisioManager.client.reportNetworkType(NetworkType.UNKNOWN) } catch (_: Exception) {}
            }
        }
        connectivityManager.registerNetworkCallback(request, callback)
        networkCallback = callback
    }

    private fun reportCurrentNetworkType() {
        val caps = connectivityManager.getNetworkCapabilities(connectivityManager.activeNetwork)
        val type = when {
            caps == null -> NetworkType.UNKNOWN
            caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> NetworkType.WIFI
            caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> NetworkType.CELLULAR
            else -> NetworkType.UNKNOWN
        }
        try { VisioManager.client.reportNetworkType(type) } catch (_: Exception) {}
    }

    private var sensorEventCount = 0L

    private fun startMotionDetection() {
        val accelerometer = sensorManager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)
        if (accelerometer == null) {
            Log.w(TAG, "No accelerometer sensor available")
            return
        }
        Log.d(TAG, "Accelerometer found: ${accelerometer.name}")
        val listener = object : SensorEventListener {
            override fun onSensorChanged(event: SensorEvent) {
                sensorEventCount++
                val x = event.values[0]
                val y = event.values[1]
                val z = event.values[2]
                val magnitude = kotlin.math.sqrt((x * x + y * y + z * z).toDouble()).toFloat()
                val deviation = kotlin.math.abs(magnitude - SensorManager.GRAVITY_EARTH)
                val now = System.currentTimeMillis()

                // Any significant acceleration extends the "moving" window
                if (deviation > MOTION_THRESHOLD) {
                    val wasMoving = lastSignificantMotionMs > 0L &&
                        now - lastSignificantMotionMs < MOTION_COOLDOWN_MS
                    lastSignificantMotionMs = now
                    if (!wasMoving && !lastReportedMotion) {
                        // First motion event — switch to moving
                        lastReportedMotion = true
                        Log.d(TAG, "Motion detected (dev=${"%.2f".format(deviation)})")
                        try { VisioManager.client.reportMotionDetected(true) } catch (_: Exception) {}
                    }
                } else if (lastReportedMotion && lastSignificantMotionMs > 0L &&
                    now - lastSignificantMotionMs > MOTION_COOLDOWN_MS) {
                    // Cooldown expired — no significant motion for 10s
                    lastReportedMotion = false
                    Log.d(TAG, "Motion stopped (${MOTION_COOLDOWN_MS}ms cooldown expired)")
                    try { VisioManager.client.reportMotionDetected(false) } catch (_: Exception) {}
                }

                if (sensorEventCount % 200 == 0L) {
                    Log.d(TAG, "Accel #$sensorEventCount: dev=${"%.2f".format(deviation)} moving=$lastReportedMotion")
                }
            }

            override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
        }
        sensorManager.registerListener(listener, accelerometer, SensorManager.SENSOR_DELAY_NORMAL)
        accelerometerListener = listener
    }

    private fun startBluetoothMonitoring() {
        val callback = object : AudioDeviceCallback() {
            override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>) {
                Log.d(TAG, "Audio devices added: ${addedDevices.map { "${it.productName}(${it.type})" }}")
                reportBluetoothCarKit()
            }
            override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) {
                Log.d(TAG, "Audio devices removed: ${removedDevices.map { "${it.productName}(${it.type})" }}")
                reportBluetoothCarKit()
            }
        }
        audioManager.registerAudioDeviceCallback(callback, android.os.Handler(android.os.Looper.getMainLooper()))
        audioDeviceCallback = callback
    }

    private fun reportBluetoothCarKit() {
        val btManager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
        val adapter = btManager?.adapter
        if (adapter == null) {
            Log.d(TAG, "No Bluetooth adapter")
            try { VisioManager.client.reportBluetoothCarKit(false) } catch (_: Exception) {}
            return
        }

        // Check connected Bluetooth audio devices via A2DP and Headset profiles
        var hasCarKit = false
        val profilesToCheck = listOf(BluetoothProfile.A2DP, BluetoothProfile.HEADSET)

        // getConnectedDevices needs BLUETOOTH_CONNECT permission
        try {
            for (profileType in profilesToCheck) {
                adapter.getProfileProxy(context, object : BluetoothProfile.ServiceListener {
                    override fun onServiceConnected(profile: Int, proxy: BluetoothProfile) {
                        try {
                            val devices = proxy.connectedDevices
                            for (device in devices) {
                                val deviceClass = device.bluetoothClass?.deviceClass ?: 0
                                val majorClass = device.bluetoothClass?.majorDeviceClass ?: 0
                                val name = device.name ?: "unknown"
                                Log.d(TAG, "BT device: name=$name class=0x${deviceClass.toString(16)} major=0x${majorClass.toString(16)}")

                                val isCarAudio = deviceClass == BluetoothClass.Device.AUDIO_VIDEO_CAR_AUDIO ||
                                    deviceClass == BluetoothClass.Device.AUDIO_VIDEO_HANDSFREE
                                if (isCarAudio) {
                                    hasCarKit = true
                                    Log.d(TAG, "Car audio device detected: $name")
                                }
                            }
                            Log.d(TAG, "Bluetooth car kit (profile=$profile): $hasCarKit")
                            VisioManager.client.reportBluetoothCarKit(hasCarKit)
                        } catch (e: SecurityException) {
                            Log.w(TAG, "BLUETOOTH_CONNECT permission not granted: ${e.message}")
                            try { VisioManager.client.reportBluetoothCarKit(false) } catch (_: Exception) {}
                        } catch (_: Exception) {}
                        try { adapter.closeProfileProxy(profile, proxy) } catch (_: Exception) {}
                    }

                    override fun onServiceDisconnected(profile: Int) {}
                }, profileType)
            }
        } catch (e: SecurityException) {
            Log.w(TAG, "Missing BLUETOOTH_CONNECT permission: ${e.message}")
            try { VisioManager.client.reportBluetoothCarKit(false) } catch (_: Exception) {}
        }
    }
}
