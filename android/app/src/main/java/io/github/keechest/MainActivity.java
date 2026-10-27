package io.github.keechest;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipDescription;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.os.PersistableBundle;
import android.provider.OpenableColumns;
import android.text.InputType;
import android.util.Log;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

/**
 * The activity around the Rust app (libkeechest_android.so). It adds what a
 * plain NativeActivity lacks: the system file picker, a clipboard that hides
 * and clears secrets, and text input from the on-screen keyboard. The Rust
 * side calls the public methods over JNI; the native methods call back.
 */
public class MainActivity extends NativeActivity {
    private static final String TAG = "KeeChest";
    private static final String CLIP_LABEL = "KeeChest";

    static {
        // Loaded here as well as by NativeActivity so that the native
        // methods below resolve.
        System.loadLibrary("keechest_android");
    }

    static native void nativeFileResult(int request, int fd, String nameOrError);

    static native void nativeText(String text);

    static native void nativeDelete(int count);

    static native void nativeEnter();

    static native void nativeBackground();

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final Runnable clearClipboard = this::clearSecretNow;
    private InputView input;
    /** A picker is in front of us: pausing for it is not leaving the app. */
    private boolean picking;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // Keep secrets out of screenshots, screen recordings and the
        // recent apps preview.
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
        super.onCreate(savedInstanceState);
        input = new InputView(this);
        addContentView(input, new ViewGroup.LayoutParams(1, 1));
    }

    @Override
    protected void onPause() {
        super.onPause();
        if (!picking) {
            nativeBackground();
        }
    }

    // File picker (Storage Access Framework).

    public void openDocument(final int request, final String mime) {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType(mime);
            startPicker(intent, request);
        });
    }

    public void createDocument(final int request, final String mime, final String name) {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType(mime);
            intent.putExtra(Intent.EXTRA_TITLE, name);
            startPicker(intent, request);
        });
    }

    private void startPicker(Intent intent, int request) {
        try {
            picking = true;
            startActivityForResult(intent, request);
        } catch (Exception e) {
            picking = false;
            Log.w(TAG, "No file picker", e);
            nativeFileResult(request, -2, "Нет приложения для выбора файлов.");
        }
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        picking = false;
        Uri uri = data == null ? null : data.getData();
        if (result != RESULT_OK || uri == null) {
            nativeFileResult(request, -1, null);
            return;
        }
        boolean create = Intent.ACTION_CREATE_DOCUMENT.equals(data.getAction());
        try {
            ParcelFileDescriptor pfd = openDescriptor(uri, create);
            nativeFileResult(request, pfd.detachFd(), displayName(uri));
        } catch (Exception e) {
            Log.w(TAG, "Can't open " + uri, e);
            nativeFileResult(request, -2, String.valueOf(e.getMessage()));
        }
    }

    private ParcelFileDescriptor openDescriptor(Uri uri, boolean write) throws Exception {
        if (!write) {
            return getContentResolver().openFileDescriptor(uri, "r");
        }
        try {
            // Truncate, in case the user picked an existing file.
            return getContentResolver().openFileDescriptor(uri, "wt");
        } catch (IllegalArgumentException | UnsupportedOperationException e) {
            return getContentResolver().openFileDescriptor(uri, "w");
        }
    }

    private String displayName(Uri uri) {
        try (Cursor c = getContentResolver().query(
                uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst() && !c.isNull(0)) {
                return c.getString(0);
            }
        } catch (Exception e) {
            Log.w(TAG, "No name for " + uri, e);
        }
        String last = uri.getLastPathSegment();
        return last == null ? "" : last;
    }

    // Clipboard.

    /**
     * Copies a secret, marked sensitive so Android 13+ doesn't show it in the
     * clipboard preview, and clears it after {@code clearAfterMs} (0 never).
     * The timer runs here so it fires even while the app is in the
     * background.
     */
    public void copySecret(final String text, final long clearAfterMs) {
        runOnUiThread(() -> {
            ClipboardManager cm = getSystemService(ClipboardManager.class);
            ClipData clip = ClipData.newPlainText(CLIP_LABEL, text);
            PersistableBundle extras = new PersistableBundle();
            // ClipDescription.EXTRA_IS_SENSITIVE, API 33.
            extras.putBoolean("android.content.extra.IS_SENSITIVE", true);
            clip.getDescription().setExtras(extras);
            cm.setPrimaryClip(clip);
            handler.removeCallbacks(clearClipboard);
            if (clearAfterMs > 0) {
                handler.postDelayed(clearClipboard, clearAfterMs);
            }
        });
    }

    /** Clears the clipboard if it still holds what we copied. */
    public void clearSecret() {
        runOnUiThread(this::clearSecretNow);
    }

    private void clearSecretNow() {
        handler.removeCallbacks(clearClipboard);
        ClipboardManager cm = getSystemService(ClipboardManager.class);
        // In the background Android 10+ hides the clipboard from us (null);
        // clear it anyway rather than leave a password there.
        ClipDescription d = cm.getPrimaryClipDescription();
        if (d != null && !CLIP_LABEL.contentEquals(String.valueOf(d.getLabel()))) {
            return;
        }
        if (Build.VERSION.SDK_INT >= 28) {
            cm.clearPrimaryClip();
        } else {
            cm.setPrimaryClip(ClipData.newPlainText("", ""));
        }
    }

    // Keyboard.

    /**
     * Shows or hides the on-screen keyboard. {@code restart} starts a new
     * input session, for a different field.
     */
    public void showKeyboard(final boolean visible, final boolean password, final boolean restart) {
        runOnUiThread(() -> {
            InputMethodManager imm = getSystemService(InputMethodManager.class);
            if (visible) {
                input.password = password;
                input.requestFocus();
                if (restart) {
                    imm.restartInput(input);
                }
                imm.showSoftInput(input, 0);
            } else {
                imm.hideSoftInputFromWindow(input.getWindowToken(), 0);
            }
        });
    }

    /**
     * An invisible view that receives the keyboard's text. NativeActivity
     * only gets key events, and keyboards send those for few characters (no
     * Cyrillic, for one), so text comes through an InputConnection instead
     * and goes to the app as text and deletions. Keys the keyboard sends as
     * key events (Backspace on an empty field, Enter) still reach the app
     * through the native input queue.
     */
    private static final class InputView extends View {
        boolean password;

        InputView(Context context) {
            super(context);
            setFocusable(true);
            setFocusableInTouchMode(true);
        }

        @Override
        public boolean onCheckIsTextEditor() {
            return true;
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo info) {
            info.inputType = InputType.TYPE_CLASS_TEXT
                    | (password
                            ? InputType.TYPE_TEXT_VARIATION_PASSWORD
                            : InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
            // No learning from what is typed: these are passwords and logins.
            info.imeOptions = EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
                    | EditorInfo.IME_FLAG_NO_FULLSCREEN
                    | EditorInfo.IME_FLAG_NO_EXTRACT_UI
                    | EditorInfo.IME_ACTION_DONE;
            return new TextConnection(this);
        }
    }

    /**
     * Turns the keyboard's edits into text and deletions. The app doesn't
     * show composing text separately, so composing text is typed in as it
     * changes and replaced (deleted and typed again) on each change.
     */
    private static final class TextConnection extends BaseInputConnection {
        /** Length in code points of the composing text the app has. */
        private int composing;

        TextConnection(View view) {
            super(view, false);
        }

        private void replaceComposing(CharSequence text) {
            String s = text.toString();
            if (composing > 0) {
                nativeDelete(composing);
            }
            if (!s.isEmpty()) {
                nativeText(s);
            }
            composing = s.codePointCount(0, s.length());
        }

        @Override
        public boolean setComposingText(CharSequence text, int newCursorPosition) {
            replaceComposing(text);
            return true;
        }

        @Override
        public boolean commitText(CharSequence text, int newCursorPosition) {
            replaceComposing(text);
            composing = 0;
            return true;
        }

        @Override
        public boolean finishComposingText() {
            composing = 0;
            return true;
        }

        @Override
        public boolean deleteSurroundingText(int beforeLength, int afterLength) {
            if (composing > 0) {
                int n = Math.min(composing, beforeLength);
                composing -= n;
            }
            if (beforeLength > 0) {
                nativeDelete(beforeLength);
            }
            return true;
        }

        @Override
        public boolean performEditorAction(int action) {
            nativeEnter();
            return true;
        }
    }
}
