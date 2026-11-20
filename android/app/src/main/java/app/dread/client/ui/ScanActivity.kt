package app.dread.client.ui

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.ImageFormat
import android.graphics.SurfaceTexture
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.ImageReader
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.util.Size
import android.view.Gravity
import android.view.Surface
import android.view.TextureView
import android.view.ViewGroup
import android.widget.FrameLayout
import app.dread.client.ui.Ui.dp
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer

/** Camera QR scanner (Camera2 + ZXing), returns the decoded text. */
class ScanActivity : Activity() {
    private lateinit var texture: TextureView
    private var camera: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var reader: ImageReader? = null
    private val thread = HandlerThread("dread-scan").apply { start() }
    private val bg = Handler(thread.looper)
    @Volatile private var done = false
    private val zxing = MultiFormatReader().apply {
        setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE), DecodeHintType.TRY_HARDER to true))
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val root = FrameLayout(this)
        texture = TextureView(this)
        root.addView(texture, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        val frame = android.view.View(this).apply { background = Ui.box(android.graphics.Color.TRANSPARENT, Ui.TEXT, 0f, context) }
        root.addView(frame, FrameLayout.LayoutParams(dp(240), dp(240), Gravity.CENTER))
        val hint = Ui.text(this, "Point the camera at the QR code on the computer's Connection page", 14f).apply {
            gravity = Gravity.CENTER
            setBackgroundColor(Ui.BG)
            setPadding(dp(16), dp(12), dp(16), dp(12))
        }
        root.addView(hint, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM))
        root.addView(Ui.button(this, "Cancel") { finish() }, FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP or Gravity.END).apply { setMargins(0, dp(16), dp(16), 0) })
        setContentView(root)
    }

    override fun onResume() {
        super.onResume()
        if (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.CAMERA), 1)
            return
        }
        if (texture.isAvailable) open() else texture.surfaceTextureListener = object : TextureView.SurfaceTextureListener {
            override fun onSurfaceTextureAvailable(s: SurfaceTexture, w: Int, h: Int) = open()
            override fun onSurfaceTextureSizeChanged(s: SurfaceTexture, w: Int, h: Int) {}
            override fun onSurfaceTextureDestroyed(s: SurfaceTexture) = true
            override fun onSurfaceTextureUpdated(s: SurfaceTexture) {}
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        if (grantResults.firstOrNull() != PackageManager.PERMISSION_GRANTED) {
            android.widget.Toast.makeText(this, "Camera permission is needed to scan. You can enter the address manually instead.", android.widget.Toast.LENGTH_LONG).show()
            finish()
        }
    }

    override fun onPause() {
        super.onPause()
        runCatching { session?.close() }
        runCatching { camera?.close() }
        runCatching { reader?.close() }
        camera = null
        session = null
    }

    override fun onDestroy() {
        thread.quitSafely()
        super.onDestroy()
    }

    @Suppress("MissingPermission")
    private fun open() {
        val cm = getSystemService(Context.CAMERA_SERVICE) as CameraManager
        val id = cm.cameraIdList.firstOrNull { cm.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_BACK }
            ?: cm.cameraIdList.firstOrNull() ?: run { finish(); return }
        val chars = cm.getCameraCharacteristics(id)
        val map = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)!!
        val size = map.getOutputSizes(ImageFormat.YUV_420_888).filter { it.width <= 1280 && it.height <= 960 }.maxByOrNull { it.width * it.height } ?: Size(640, 480)
        val r = ImageReader.newInstance(size.width, size.height, ImageFormat.YUV_420_888, 2)
        reader = r
        r.setOnImageAvailableListener({ ir ->
            val img = ir.acquireLatestImage() ?: return@setOnImageAvailableListener
            try {
                if (done) return@setOnImageAvailableListener
                val plane = img.planes[0]
                val w = img.width
                val h = img.height
                val rowStride = plane.rowStride
                val buf = plane.buffer
                val data = ByteArray(rowStride * h)
                buf.get(data, 0, minOf(data.size, buf.remaining()))
                val src = PlanarYUVLuminanceSource(data, rowStride, h, 0, 0, w, h, false)
                val res = runCatching { zxing.decodeWithState(BinaryBitmap(HybridBinarizer(src))) }.getOrNull()
                zxing.reset()
                if (res != null) {
                    done = true
                    runOnUiThread {
                        setResult(RESULT_OK, Intent().putExtra(EXTRA_TEXT, res.text))
                        finish()
                    }
                }
            } finally {
                img.close()
            }
        }, bg)
        cm.openCamera(id, object : CameraDevice.StateCallback() {
            override fun onOpened(cd: CameraDevice) {
                camera = cd
                val st = texture.surfaceTexture ?: return
                st.setDefaultBufferSize(size.width, size.height)
                val preview = Surface(st)
                val req = cd.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW).apply {
                    addTarget(preview)
                    addTarget(r.surface)
                    set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
                }
                @Suppress("DEPRECATION")
                cd.createCaptureSession(listOf(preview, r.surface), object : CameraCaptureSession.StateCallback() {
                    override fun onConfigured(s: CameraCaptureSession) {
                        session = s
                        runCatching { s.setRepeatingRequest(req.build(), null, bg) }
                    }
                    override fun onConfigureFailed(s: CameraCaptureSession) { finish() }
                }, bg)
            }
            override fun onDisconnected(cd: CameraDevice) { cd.close() }
            override fun onError(cd: CameraDevice, error: Int) { cd.close(); finish() }
        }, bg)
    }

    companion object {
        const val EXTRA_TEXT = "text"
    }
}
