package io.visio.mobile

import android.bluetooth.BluetoothDevice
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
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
    private var bluetoothReceiver: BroadcastReceiver? = null

    private var lastAccelTimestamp = 0L
    private var motionCount = 0
    private var lastReportedMotion = false
    private val MOTION_THRESHOLD = 1.2f
    private val MOTION_WINDOW_MS = 3000L
    private val MOTION_COUNT_THRESHOLD = 8

    companion object {
        private const val TAG = "ContextDetector"
    }

    fun start() {
        startNetworkMonitoring()
        startMotionDetection()
        startBluetoothMonitoring()
        reportCurrentNetworkType()
        reportBluetoothCarKit()
    }

    fun stop() {
        networkCallback?.let { connectivityManager.unregisterNetworkCallback(it) }
        accelerometerListener?.let { sensorManager.unregisterListener(it) }
        bluetoothReceiver?.let {
            try { context.unregisterReceiver(it) } catch (_: Exception) {}
        }
        networkCallback = null
        accelerometerListener = null
        bluetoothReceiver = null
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

    private fun startMotionDetection() {
        val accelerometer = sensorManager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER) ?: return
        val listener = object : SensorEventListener {
            override fun onSensorChanged(event: SensorEvent) {
                val x = event.values[0]
                val y = event.values[1]
                val z = event.values[2]
                val magnitude = kotlin.math.sqrt((x * x + y * y + z * z).toDouble()).toFloat()
                val deviation = kotlin.math.abs(magnitude - SensorManager.GRAVITY_EARTH)

                val now = System.currentTimeMillis()
                if (now - lastAccelTimestamp > MOTION_WINDOW_MS) {
                    motionCount = 0
                    lastAccelTimestamp = now
                }

                if (deviation > MOTION_THRESHOLD) {
                    motionCount++
                }

                val moving = motionCount > MOTION_COUNT_THRESHOLD
                if (moving != lastReportedMotion) {
                    lastReportedMotion = moving
                    Log.d(TAG, "Motion state changed: moving=$moving (count=$motionCount)")
                    try { VisioManager.client.reportMotionDetected(moving) } catch (_: Exception) {}
                }
            }

            override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
        }
        sensorManager.registerListener(listener, accelerometer, SensorManager.SENSOR_DELAY_NORMAL)
        accelerometerListener = listener
    }

    private fun startBluetoothMonitoring() {
        val filter = IntentFilter().apply {
            addAction(BluetoothDevice.ACTION_ACL_CONNECTED)
            addAction(BluetoothDevice.ACTION_ACL_DISCONNECTED)
        }
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context, intent: Intent) {
                reportBluetoothCarKit()
            }
        }
        context.registerReceiver(receiver, filter)
        bluetoothReceiver = receiver
    }

    private fun reportBluetoothCarKit() {
        val devices = audioManager.getDevices(AudioManager.GET_DEVICES_OUTPUTS)
        val hasCarKit = devices.any {
            it.type == AudioDeviceInfo.TYPE_BLUETOOTH_A2DP ||
            it.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO
        }
        try { VisioManager.client.reportBluetoothCarKit(hasCarKit) } catch (_: Exception) {}
    }
}
