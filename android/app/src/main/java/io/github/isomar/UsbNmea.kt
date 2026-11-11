package io.github.isomar

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.hoho.android.usbserial.driver.UsbSerialDriver
import com.hoho.android.usbserial.driver.UsbSerialPort
import com.hoho.android.usbserial.driver.UsbSerialProber
import com.hoho.android.usbserial.util.SerialInputOutputManager

/**
 * NMEA 0183 from a USB serial adapter (an instrument bus, a GPS, an AIS receiver): the engine's serial
 * connection kind has no port to open on Android, so the app reads the adapter itself and hands the engine
 * its lines (`Native.nmea`, source "usb"), on the main thread.
 */
object UsbNmea {
    private const val PERMISSION = "eu.sixpx.isomar.USB_PERMISSION"
    private val main = Handler(Looper.getMainLooper())
    private lateinit var app: Context
    private var port: UsbSerialPort? = null
    private var reader: SerialInputOutputManager? = null
    private val line = StringBuilder()
    private var wanted: Pair<UsbDevice, Int>? = null

    /** The adapters plugged in, by name. */
    var devices by mutableStateOf(emptyList<String>())
        private set
    /** The adapter being read, and at what speed. */
    var open by mutableStateOf<String?>(null)
        private set
    var baud by mutableStateOf(4800)

    fun init(context: Context) {
        app = context.applicationContext
        val filter = IntentFilter().apply {
            addAction(PERMISSION)
            addAction(UsbManager.ACTION_USB_DEVICE_DETACHED)
            addAction(UsbManager.ACTION_USB_DEVICE_ATTACHED)
        }
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(c: Context, i: Intent) {
                when (i.action) {
                    PERMISSION -> wanted?.let { (d, b) -> if (manager.hasPermission(d)) start(d, b) else Model.show("Isomar may not use that USB device") }
                    UsbManager.ACTION_USB_DEVICE_DETACHED -> { if (open != null && drivers().none { name(it) == open }) close(); refresh() }
                    else -> refresh()
                }
            }
        }
        if (Build.VERSION.SDK_INT >= 33) app.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED) else app.registerReceiver(receiver, filter)
        refresh()
    }

    private val manager get() = app.getSystemService(Context.USB_SERVICE) as UsbManager

    private fun drivers(): List<UsbSerialDriver> = runCatching { UsbSerialProber.getDefaultProber().findAllDrivers(manager) }.getOrDefault(emptyList())

    private fun name(d: UsbSerialDriver): String = d.device.productName?.takeIf { it.isNotBlank() } ?: d.device.deviceName

    fun refresh() {
        devices = drivers().map(::name)
    }

    /** Opens the adapter called `name` (asking the user first, the first time) at `baud`. */
    fun connect(name: String, baud: Int = this.baud) {
        val d = drivers().firstOrNull { name(it) == name } ?: return
        this.baud = baud
        if (manager.hasPermission(d.device)) { start(d.device, baud); return }
        wanted = d.device to baud
        val flags = if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0
        manager.requestPermission(d.device, PendingIntent.getBroadcast(app, 0, Intent(PERMISSION).setPackage(app.packageName), flags))
    }

    /** A device just plugged in (the activity was opened for it): read it at once. */
    fun attached(device: UsbDevice) {
        val d = drivers().firstOrNull { it.device.deviceId == device.deviceId } ?: return
        refresh()
        connect(name(d))
    }

    private fun start(device: UsbDevice, baud: Int) {
        close()
        val driver = drivers().firstOrNull { it.device.deviceId == device.deviceId } ?: return
        val conn = manager.openDevice(device) ?: run { Model.show("Could not open the USB device"); return }
        val p = driver.ports.firstOrNull() ?: return
        runCatching {
            p.open(conn)
            p.setParameters(baud, 8, UsbSerialPort.STOPBITS_1, UsbSerialPort.PARITY_NONE)
        }.onFailure { Model.show("Could not open the USB device: ${it.message}"); runCatching { p.close() }; return }
        port = p
        open = name(driver)
        reader = SerialInputOutputManager(p, object : SerialInputOutputManager.Listener {
            override fun onNewData(data: ByteArray) = received(data)
            override fun onRunError(e: Exception) {
                main.post { close(); Model.show("The USB device stopped: ${e.message ?: ""}") }
            }
        }).also { it.start() }
        Model.show("Reading ${open} at $baud baud")
    }

    /** On the reader's thread: cut into lines, each handed to the engine on the main thread. */
    private fun received(data: ByteArray) {
        val done = ArrayList<String>()
        synchronized(line) {
            for (b in data) {
                val ch = (b.toInt() and 0xff).toChar()
                when (ch) {
                    '\n' -> { val l = line.toString().trim(); line.setLength(0); if (l.isNotEmpty()) done += l }
                    '\r' -> {}
                    else -> if (line.length < 200) line.append(ch) else line.setLength(0)
                }
            }
        }
        if (done.isNotEmpty()) main.post { for (l in done) Native.nmea("usb", l) }
    }

    fun close() {
        reader?.stop()
        reader = null
        runCatching { port?.close() }
        port = null
        open = null
    }
}
