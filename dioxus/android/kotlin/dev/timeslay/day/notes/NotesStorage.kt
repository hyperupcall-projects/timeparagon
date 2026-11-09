package dev.timeslay.day.notes

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.DocumentsContract as Docs
import org.json.JSONArray
import org.json.JSONObject

/** User-granted document trees, never broad external-storage permissions. */
object NotesStorage {
    @JvmStatic fun run(context: Context, input: String): String = try {
        val request = JSONObject(input)
        val prefs = context.getSharedPreferences("notes-picker", Context.MODE_PRIVATE)
        val result = JSONObject()
        when (request.getString("op")) {
            "pick" -> {
                prefs.edit().remove("selection").apply()
                context.startActivity(Intent(context, NotesFolderActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            "selection" -> {
                result.put("directory", prefs.getString("selection", null))
                result.put("error", prefs.getString("error", null))
                prefs.edit().remove("selection").remove("error").apply()
            }
            else -> {
                val tree = Uri.parse(request.getString("tree"))
                require(tree.scheme == "content") { "Choose a notes folder in Settings first" }
                val resolver = context.contentResolver
                val root = Docs.buildDocumentUriUsingTree(tree, Docs.getTreeDocumentId(tree))
                val children = Docs.buildChildDocumentsUriUsingTree(tree, Docs.getTreeDocumentId(tree))
                val files = sortedMapOf<String, Uri>()
                val rows = JSONArray()
                resolver.query(children, arrayOf(Docs.Document.COLUMN_DOCUMENT_ID, Docs.Document.COLUMN_DISPLAY_NAME, Docs.Document.COLUMN_LAST_MODIFIED, Docs.Document.COLUMN_SIZE, Docs.Document.COLUMN_MIME_TYPE), null, null, null)?.use { cursor ->
                    while (cursor.moveToNext()) {
                        val name = cursor.getString(1)
                        if (!name.startsWith(".") && name.endsWith(".md", true) && cursor.getString(4) != Docs.Document.MIME_TYPE_DIR) {
                            val uri = Docs.buildDocumentUriUsingTree(tree, cursor.getString(0))
                            files[name] = uri
                            rows.put(JSONObject().put("name", name).put("modified", cursor.getLong(2)).put("size", cursor.getLong(3)))
                        }
                    }
                } ?: error("Could not read the notes folder")
                when (request.getString("op")) {
                    "stat" -> result.put("files", rows)
                    "read" -> {
                        for (i in 0 until rows.length()) {
                            val row = rows.getJSONObject(i)
                            row.put("text", resolver.openInputStream(files.getValue(row.getString("name")))?.bufferedReader(Charsets.UTF_8)?.use { it.readText() } ?: error("Could not read note"))
                        }
                        result.put("files", rows)
                    }
                    "write" -> {
                        val writes = request.getJSONObject("writes")
                        for (name in writes.keys()) {
                            require(!name.contains('/') && !name.contains('\\') && name.endsWith(".md", true))
                            val uri = files[name] ?: Docs.createDocument(resolver, root, "text/markdown", name) ?: error("Could not create $name")
                            resolver.openOutputStream(uri, "wt")?.use { it.write(writes.getString(name).toByteArray(Charsets.UTF_8)) } ?: error("Could not save $name")
                        }
                        // Finish writes before removing originals on rename.
                        val removed = request.getJSONArray("removed")
                        for (i in 0 until removed.length()) files[removed.getString(i)]?.let { check(Docs.deleteDocument(resolver, it)) { "Could not delete note" } }
                    }
                    "open" -> {
                        val uri = files[request.getString("file")] ?: error("Note no longer exists")
                        val intent = Intent(Intent.ACTION_VIEW).setDataAndType(uri, "text/markdown")
                            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
                        try { context.startActivity(intent) }
                        catch (_: android.content.ActivityNotFoundException) { context.startActivity(intent.setDataAndType(uri, "text/plain")) }
                    }
                }
            }
        }
        result.toString()
    } catch (error: Exception) { JSONObject().put("error", error.message ?: "Notes storage failed").toString() }
}

class NotesFolderActivity : Activity() {
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        if (state == null) startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION), 1)
    }
    @Deprecated("Framework activity result API")
    override fun onActivityResult(request: Int, result: Int, data: Intent?) {
        super.onActivityResult(request, result, data)
        if (request == 1 && result == RESULT_OK && data?.data != null) {
            val prefs = getSharedPreferences("notes-picker", MODE_PRIVATE)
            try {
                val uri = data.data!!
                contentResolver.takePersistableUriPermission(uri, data.flags and (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION))
                prefs.edit().putString("selection", uri.toString()).apply()
            } catch (error: Exception) { prefs.edit().putString("error", error.message).apply() }
        }
        finish()
    }
}
