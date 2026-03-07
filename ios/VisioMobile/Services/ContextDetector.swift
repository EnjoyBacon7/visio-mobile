import Foundation
import Network
import CoreMotion
import AVFoundation
import visioFFI

class ContextDetector {
    private let pathMonitor = NWPathMonitor()
    private let motionManager = CMMotionActivityManager()
    private let monitorQueue = DispatchQueue(label: "io.visio.context")

    private var isMoving = false

    func start() {
        startNetworkMonitoring()
        startMotionDetection()
        startBluetoothMonitoring()
    }

    func stop() {
        pathMonitor.cancel()
        motionManager.stopActivityUpdates()
        NotificationCenter.default.removeObserver(self)
    }

    private func startNetworkMonitoring() {
        pathMonitor.pathUpdateHandler = { [weak self] path in
            guard self != nil else { return }
            let type: NetworkType
            if path.usesInterfaceType(.wifi) {
                type = .wifi
            } else if path.usesInterfaceType(.cellular) {
                type = .cellular
            } else {
                type = .unknown
            }
            DispatchQueue.main.async {
                VisioManager.shared.client.reportNetworkType(networkType: type)
            }
        }
        pathMonitor.start(queue: monitorQueue)
    }

    private func startMotionDetection() {
        guard CMMotionActivityManager.isActivityAvailable() else { return }
        motionManager.startActivityUpdates(to: .main) { [weak self] activity in
            guard let activity = activity else { return }
            let moving = activity.walking || activity.running || activity.cycling
            guard moving != self?.isMoving else { return }
            self?.isMoving = moving
            VisioManager.shared.client.reportMotionDetected(detected: moving)
        }
    }

    private func startBluetoothMonitoring() {
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(audioRouteChanged),
            name: AVAudioSession.routeChangeNotification,
            object: nil
        )
        reportBluetoothCarKit()
    }

    @objc private func audioRouteChanged(_ notification: Notification) {
        reportBluetoothCarKit()
    }

    private func reportBluetoothCarKit() {
        let route = AVAudioSession.sharedInstance().currentRoute
        let hasCarKit = route.outputs.contains { port in
            port.portType == .bluetoothA2DP ||
            port.portType == .bluetoothHFP ||
            port.portType == .carAudio
        }
        DispatchQueue.main.async {
            VisioManager.shared.client.reportBluetoothCarKit(connected: hasCarKit)
        }
    }
}
