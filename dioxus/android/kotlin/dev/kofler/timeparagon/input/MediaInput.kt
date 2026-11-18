package dev.kofler.timeparagon.input

import android.Manifest
import android.app.Activity
import android.content.*
import android.content.pm.PackageManager
import android.database.Cursor
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.net.Uri
import android.os.*
import android.provider.MediaStore
import org.json.JSONObject
import java.io.File
import java.io.RandomAccessFile
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.sqrt

/** Only acquisition lives here. Rust owns model calls and draft changes. */
object MediaInput {
    private var id = ""
    private var state = JSONObject()
    private var file: File? = null
    private var stop = AtomicBoolean(false)
    @Synchronized fun current(token: String) = token == id
    @Synchronized fun result(token: String, value: JSONObject) { if (current(token)) state = value }
    fun fail(token: String, message: String) = result(token, JSONObject().put("error", message))
    @Synchronized fun output(context: Context, token: String, suffix: String): File {
        check(current(token)) { "Input cancelled" }
        return File.createTempFile("ai-input-", suffix, context.cacheDir).also { file = it }
    }
    @JvmStatic @Synchronized fun run(context: Context, input: String): String = try {
        val request = JSONObject(input)
        val token = request.getString("id")
        when (request.getString("op")) {
            "start" -> {
                stop.set(true)
                file?.delete()
                file = null
                id = token
                state = JSONObject().put("phase", "Opening input")
                context.startActivity(Intent(context, MediaInputActivity::class.java)
                    .putExtra("id", token).putExtra("source", request.getString("source"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            "copy" -> (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("Model debug", request.getString("text")))
            "poll" -> Unit
            "stop" -> if (current(token)) stop.set(true)
            "cancel" -> if (current(token)) {
                stop.set(true); file?.delete(); file = null; id = ""; state = JSONObject()
            }
        }
        if (request.getString("op") == "poll") { if (current(token)) state.toString() else JSONObject().put("cancelled", true).toString() } else JSONObject().toString()
    } catch (error: Exception) { JSONObject().put("error", error.message ?: "Input failed").toString() }

    fun record(context: Context, token: String) {
        val target = output(context, token, ".wav")
        val stopping = AtomicBoolean(false)
        synchronized(this) { if (!current(token)) return; stop = stopping }
        Thread {
            var recorder: AudioRecord? = null
            try {
                val size = maxOf(3200, AudioRecord.getMinBufferSize(16000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT))
                recorder = AudioRecord(MediaRecorder.AudioSource.MIC, 16000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, size * 2)
                check(recorder.state == AudioRecord.STATE_INITIALIZED) { "Cannot open microphone" }
                val samples = ShortArray(800)
                val pcm = ByteArray(1600)
                var count = 0
                var heard = false
                RandomAccessFile(target, "rw").use { wav ->
                    wav.write(ByteArray(44))
                    recorder.startRecording()
                    val started = SystemClock.elapsedRealtime()
                    while (!stopping.get() && current(token) && count < 16000 * 120 && SystemClock.elapsedRealtime() - started < 120000) {
                        val n = recorder.read(samples, 0, minOf(samples.size, 16000 * 120 - count), AudioRecord.READ_NON_BLOCKING)
                        check(n >= 0) { "Microphone disconnected" }
                        if (n == 0) { Thread.sleep(20); continue }
                        var energy = 0.0
                        for (i in 0 until n) {
                            val sample = samples[i].toInt()
                            pcm[i * 2] = sample.toByte(); pcm[i * 2 + 1] = (sample shr 8).toByte()
                            energy += (sample / 32768.0) * (sample / 32768.0)
                        }
                        wav.write(pcm, 0, n * 2)
                        count += n
                        val rms = sqrt(energy / n)
                        heard = heard || rms > 0.01
                        result(token, JSONObject().put("phase", "Listening").put("level", ((rms - 0.01) * 8).coerceIn(0.0, 1.0)))
                    }
                    check(heard && count >= 3200) { "No speech detected; check your microphone and try again" }
                    fun le(value: Int, bytes: Int) { for (i in 0 until bytes) wav.write((value shr (8 * i)) and 255) }
                    wav.seek(0); wav.writeBytes("RIFF"); le(36 + count * 2, 4); wav.writeBytes("WAVEfmt "); le(16, 4)
                    le(1, 2); le(1, 2); le(16000, 4); le(32000, 4); le(2, 2); le(16, 2); wav.writeBytes("data"); le(count * 2, 4)
                }
                result(token, JSONObject().put("path", target.absolutePath))
            } catch (error: Exception) {
                target.delete(); fail(token, error.message ?: "Recording failed")
            } finally {
                try { recorder?.stop() } catch (_: Exception) {}
                recorder?.release()
                if (!current(token)) target.delete()
            }
        }.start()
    }
}

class MediaInputActivity : Activity() {
    private val token get() = intent.getStringExtra("id") ?: ""
    private val source get() = intent.getStringExtra("source") ?: ""
    private var camera: Uri? = null
    private var pasted = false
    override fun onCreate(saved: Bundle?) {
        super.onCreate(saved)
        camera = saved?.getString("camera")?.let(Uri::parse)
        if (!MediaInput.current(token)) { finish(); return }
        if (saved != null) return
        try {
            when (source) {
                "voice" -> if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 2) else record()
                "clipboard" -> Unit // Read only after this activity has window focus.
                "camera" -> {
                    val file = MediaInput.output(this, token, ".jpg")
                    camera = Uri.parse("content://$packageName.ai-input/${file.name}")
                    val capture = Intent(MediaStore.ACTION_IMAGE_CAPTURE).putExtra(MediaStore.EXTRA_OUTPUT, camera)
                        .addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    capture.clipData = ClipData.newRawUri("Picture", camera)
                    startActivityForResult(capture, 1)
                }
                else -> startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).setType("image/*")
                    .addCategory(Intent.CATEGORY_OPENABLE).putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("image/png", "image/jpeg", "image/webp")), 1)
            }
        } catch (error: Exception) { MediaInput.fail(token, error.message ?: "Cannot open input"); finish() }
    }
    override fun onWindowFocusChanged(focused: Boolean) {
        super.onWindowFocusChanged(focused)
        if (focused && source == "clipboard" && !pasted) { pasted = true; paste() }
    }
    @Deprecated("Framework back API")
    override fun onBackPressed() { MediaInput.result(token, JSONObject().put("cancelled", true)); super.onBackPressed() }
    override fun onSaveInstanceState(out: Bundle) { out.putString("camera", camera?.toString()); super.onSaveInstanceState(out) }
    private fun record() { try { MediaInput.record(this, token) } catch (error: Exception) { MediaInput.fail(token, error.message ?: "Cannot record") }; finish() }
    override fun onRequestPermissionsResult(request: Int, permissions: Array<out String>, grants: IntArray) {
        super.onRequestPermissionsResult(request, permissions, grants)
        if (grants.firstOrNull() == PackageManager.PERMISSION_GRANTED) record()
        else { MediaInput.fail(token, "Microphone permission denied. Enable it in Android Settings → Apps → TimeParagon → Permissions."); finish() }
    }
    private fun paste() {
        try {
            val clip = (getSystemService(CLIPBOARD_SERVICE) as ClipboardManager).primaryClip
            val uri = if (clip != null && clip.itemCount > 0) clip.getItemAt(0).uri else null
            check(uri != null && uri.scheme == "content") { "Clipboard has no picture. Copy a picture and try again." }
            readPicture(uri)
        } catch (error: Exception) { MediaInput.fail(token, error.message ?: "Cannot paste image"); finish() }
    }
    private fun readPicture(uri: Uri) {
        Thread {
            try { copy(uri) } catch (error: Exception) { MediaInput.fail(token, error.message ?: "Cannot read picture") }
            runOnUiThread { finish() }
        }.start()
    }
    private fun copy(uri: Uri) {
        if (!MediaInput.current(token)) return
        val target = MediaInput.output(this, token, ".image")
        try {
            contentResolver.openInputStream(uri)?.use { input ->
                target.outputStream().use { output ->
                    val buffer = ByteArray(8192); var total = 0
                    while (true) { val n = input.read(buffer); if (n < 0) break; total += n; check(total <= 20 * 1024 * 1024) { "Picture exceeds 20 MB" }; output.write(buffer, 0, n) }
                }
            } ?: error("Cannot read picture")
            MediaInput.result(token, JSONObject().put("path", target.absolutePath))
        } catch (error: Exception) { target.delete(); throw error }
    }
    @Deprecated("Framework activity result API")
    override fun onActivityResult(request: Int, result: Int, data: Intent?) {
        super.onActivityResult(request, result, data)
        try {
            if (result != RESULT_OK) MediaInput.result(token, JSONObject().put("cancelled", true))
            else if (source == "camera") {
                val name = camera?.lastPathSegment ?: error("Picture unavailable")
                val target = File(cacheDir, name)
                check(target.length() in 1..20L * 1024 * 1024) { "Picture must be between 1 byte and 20 MB" }
                MediaInput.result(token, JSONObject().put("path", target.absolutePath))
            } else { readPicture(data?.data ?: error("No picture selected")); return }
        } catch (error: Exception) { MediaInput.fail(token, error.message ?: "Cannot read image") }
        finish()
    }
}

/** A single temporary camera output, granted only to the camera app. */
class CameraOutput : ContentProvider() {
    override fun onCreate() = true
    override fun getType(uri: Uri) = "image/jpeg"
    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        val name = uri.lastPathSegment ?: error("Missing picture")
        require(name.startsWith("ai-input-") && name.endsWith(".jpg") && !name.contains('/') && !name.contains('\\'))
        return ParcelFileDescriptor.open(File(context!!.cacheDir, name), ParcelFileDescriptor.parseMode(mode))
    }
    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, sort: String?): Cursor? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, args: Array<out String>?) = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?) = 0
}
