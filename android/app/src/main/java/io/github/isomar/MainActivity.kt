package io.github.isomar

import android.Manifest
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.DocumentsContract
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView
import io.github.isomar.ui.IsomarTheme
import io.github.isomar.ui.IsomarScreen
import androidx.compose.runtime.LaunchedEffect
import androidx.window.layout.FoldingFeature
import androidx.window.layout.WindowInfoTracker
import java.io.File

/**
 * The window: the chart and the phone layout over it (ui/PhoneScreen.kt). The engine and the model live in
 * the Application and outlive this Activity (a fold, a rotation, the screen going off).
 */
class MainActivity : ComponentActivity() {
    private var chart: ChartView? = null
    private val gps by lazy { DeviceLocation(applicationContext) }

    private val openGpx = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> uri?.let(::importGpx) }
    private val openCharts = registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri -> uri?.let(::importCharts) }
    private val saveFile = registerForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) { uri -> uri?.let(::saveTo) }
    /** What writes the file being saved, to a temporary path. */
    private var pendingWrite: ((String) -> String?)? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        intent?.getStringExtra("sheet")?.let { Model.startSheet = it }
        // A debuggable build runs a script of steps (Script.kt), for screenshots of many states in one launch.
        if (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0 && savedInstanceState == null) {
            intent?.getStringExtra("script")?.let { Script.run(it, filesDir) }
        }
        usbAttached(intent)
        Alerting.screenshot = Screenshots.on(this)
        intent?.getStringExtra("page")?.let { p -> Page.entries.firstOrNull { it.name.equals(p, true) }?.let { Model.page = it } }
        Model.redraw = { chart?.postInvalidateOnAnimation() }
        Model.copy = { text ->
            getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Isomar", text))
        }
        Model.sound = Alerting::play
        Model.openUrl = { url -> runCatching { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) } }
        Model.pickFile = ::pick
        Model.saveAs = { name, write -> pendingWrite = write; saveFile.launch(name) }
        Alerting.keepScreenOn = { on ->
            if (on) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
        Alerting.overLockScreen = { on ->
            setShowWhenLocked(on)
            setTurnScreenOn(on)
        }
        val prefs = getSharedPreferences("phone", MODE_PRIVATE)
        Model.dimAtNight = prefs.getBoolean("dim_at_night", true)
        Model.keepOnUnderway = prefs.getBoolean("keep_on_underway", true)
        setContent {
            // Night dims the screen to its lowest, for eyes used to the dark; any other look gives it back.
            val night = io.github.isomar.ui.themeFor(Model.mode, androidx.compose.foundation.isSystemInDarkTheme()) == io.github.isomar.ui.ThemeId.Night
            LaunchedEffect(night, Model.dimAtNight, Model.keepOnUnderway) {
                prefs.edit().putBoolean("dim_at_night", Model.dimAtNight).putBoolean("keep_on_underway", Model.keepOnUnderway).apply()
                window.attributes = window.attributes.apply {
                    screenBrightness = if (night && Model.dimAtNight) 0.02f else WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE
                }
            }
            // A foldable open like a book (a vertical hinge that separates the screen): the pages go left of it.
            LaunchedEffect(Unit) {
                WindowInfoTracker.getOrCreate(this@MainActivity).windowLayoutInfo(this@MainActivity).collect { info ->
                    val fold = info.displayFeatures.filterIsInstance<FoldingFeature>()
                        .firstOrNull { it.isSeparating && it.orientation == FoldingFeature.Orientation.VERTICAL }
                    val d = resources.displayMetrics.density
                    Model.hinge = fold?.let { it.bounds.left / d }
                    Model.hingeWidth = fold?.let { it.bounds.width() / d } ?: 0f
                }
            }
            IsomarTheme {
                IsomarScreen {
                    AndroidView(factory = { ChartView(it).also { v -> chart = v } }, modifier = Modifier.fillMaxSize())
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        usbAttached(intent)
    }

    /** Opened because a USB serial adapter was plugged in: read it. */
    private fun usbAttached(i: Intent?) {
        if (i?.action != android.hardware.usb.UsbManager.ACTION_USB_DEVICE_ATTACHED) return
        val d: android.hardware.usb.UsbDevice? = if (Build.VERSION.SDK_INT >= 33) i.getParcelableExtra(android.hardware.usb.UsbManager.EXTRA_DEVICE, android.hardware.usb.UsbDevice::class.java)
        else @Suppress("DEPRECATION") i.getParcelableExtra(android.hardware.usb.UsbManager.EXTRA_DEVICE)
        d?.let(UsbNmea::attached)
    }

    override fun onStart() {
        super.onStart()
        Alerting.visible = true
        if (!Screenshots.on(this)) {
            val wanted = buildList {
                if (!gps.granted()) { add(Manifest.permission.ACCESS_FINE_LOCATION); add(Manifest.permission.ACCESS_COARSE_LOCATION) }
                if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) add(Manifest.permission.POST_NOTIFICATIONS)
            }
            if (wanted.isNotEmpty()) requestPermissions(wanted.toTypedArray(), 1)
        }
        gps.start()
    }

    override fun onStop() {
        super.onStop()
        Alerting.visible = false
        // Android may end a stopped app without telling it: the view and the settings are saved now.
        Model.send("save_state")
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        gps.start()
    }

    // Files: Android gives the app documents, not paths, so what the engine reads is copied in first.

    private fun pick(kind: String) {
        when {
            kind == "gpx" -> openGpx.launch(arrayOf("application/gpx+xml", "application/octet-stream", "text/xml", "application/xml", "*/*"))
            kind == "charts" -> openCharts.launch(null)
            kind.startsWith("gpx-export:") -> {
                val parts = kind.split(':', limit = 3)
                val id = parts.getOrNull(1)?.toLongOrNull() ?: return
                Model.saveAs((parts.getOrNull(2)?.ifEmpty { null } ?: "route") + ".gpx") { path -> Model.send("gpx_export", Model.args("id" to id, "path" to path)) }
            }
        }
    }

    private fun importGpx(uri: Uri) {
        val dir = File(cacheDir, "import").apply { mkdirs() }
        val out = File(dir, "import.gpx")
        runCatching { contentResolver.openInputStream(uri)!!.use { i -> out.outputStream().use { i.copyTo(it) } } }
            .onFailure { Model.error = ErrorRequest("Could not read the file", it.message ?: ""); return }
        Model.send("gpx_import", Model.args("path" to out.absolutePath))?.let { Model.error = ErrorRequest("Could not import the file", it) }
        out.delete()
    }

    private fun saveTo(uri: Uri) {
        val write = pendingWrite ?: return
        pendingWrite = null
        val tmp = File(cacheDir, "export.tmp")
        val e = write(tmp.absolutePath)
        if (e != null) { Model.error = ErrorRequest("Could not write the file", e); return }
        runCatching { contentResolver.openOutputStream(uri)!!.use { o -> tmp.inputStream().use { it.copyTo(o) } } }
            .onFailure { Model.error = ErrorRequest("Could not write the file", it.message ?: "") }
        tmp.delete()
    }

    /** A folder of charts (S-57 cells, BSB/KAP) is copied into the app, then added like a folder on the desktop. */
    private fun importCharts(tree: Uri) {
        val root = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
        val name = displayName(root) ?: "charts"
        val dest = File(File(filesDir, "charts"), name.replace('/', '_'))
        Model.show("Copying $name…")
        Thread {
            val ok = runCatching { copyTree(tree, DocumentsContract.getTreeDocumentId(tree), dest) }
            runOnUiThread {
                ok.onFailure { Model.error = ErrorRequest("Could not copy the charts", it.message ?: "") }
                    .onSuccess { Model.send("charts_add_folder", Model.args("path" to dest.absolutePath)); Model.showPage(Page.Charts) }
            }
        }.start()
    }

    private fun displayName(doc: Uri): String? =
        contentResolver.query(doc, arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME), null, null, null)?.use { c -> if (c.moveToFirst()) c.getString(0) else null }

    private fun copyTree(tree: Uri, docId: String, dest: File) {
        dest.mkdirs()
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, docId)
        val cols = arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE)
        contentResolver.query(children, cols, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val id = c.getString(0)
                val name = c.getString(1).replace('/', '_')
                if (c.getString(2) == DocumentsContract.Document.MIME_TYPE_DIR) {
                    copyTree(tree, id, File(dest, name))
                } else {
                    val uri = DocumentsContract.buildDocumentUriUsingTree(tree, id)
                    contentResolver.openInputStream(uri)?.use { i -> File(dest, name).outputStream().use { i.copyTo(it) } }
                }
            }
        }
    }
}

/** A debuggable build started for a screenshot asks for nothing (`adb shell am start ... --ez screenshot true`). */
object Screenshots {
    fun on(a: android.app.Activity) = a.intent?.getBooleanExtra("screenshot", false) == true
}
