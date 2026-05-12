package com.ramus.app

import android.os.Bundle
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private var webView: WebView? = null
  private var lastInsetTopCssPx: Int = 0
  private var lastInsetBottomCssPx: Int = 0
  private var hasReceivedInsets: Boolean = false

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

    // Push the system-bar insets (status bar on top, nav bar on bottom) into CSS
    // custom properties so the WebView can avoid drawing under them. Chromium's
    // env(safe-area-inset-*) ignores the 3-button nav bar height; this fills the
    // gap. We listen on the activity's content view rather than the WebView
    // itself — wry's WebView often consumes or doesn't receive insets cleanly
    // when added as the activity's content, so the parent FrameLayout is more
    // reliable. The listener also fires on rotation, IME show/hide, and
    // nav-mode changes, keeping the variables live.
    val content = findViewById<android.view.View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { _, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
      val density = resources.displayMetrics.density.coerceAtLeast(0.1f)
      lastInsetTopCssPx = (bars.top / density).toInt()
      lastInsetBottomCssPx = (bars.bottom / density).toInt()
      hasReceivedInsets = true
      pushInsetsToWeb()
      insets
    }

    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        val wv = webView
        if (wv != null && wv.isAttachedToWindow) {
          wv.evaluateJavascript(
            "(function(){var e=new CustomEvent('android-back-button',{cancelable:true});" +
              "window.dispatchEvent(e);return e.defaultPrevented;})()"
          ) { handled ->
            if (handled != "true") {
              moveTaskToBack(true)
            }
          }
        } else {
          moveTaskToBack(true)
        }
      }
    })
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    // The page is almost certainly not loaded yet when this fires, so the first
    // evaluateJavascript would silently no-op. Re-push on a short delay (well
    // after Vite serves the document) and again 1s later as belt-and-braces.
    webView.postDelayed({ pushInsetsToWeb() }, 200)
    webView.postDelayed({ pushInsetsToWeb() }, 1000)
    // Trigger an insets pass in case onCreate's listener was attached after the
    // initial layout dispatched them.
    ViewCompat.requestApplyInsets(findViewById(android.R.id.content))
  }

  override fun onResume() {
    super.onResume()
    // Nav mode (gesture vs 3-button) can change while we're backgrounded.
    // Request insets first so the listener overwrites the cached values, then
    // push — the listener's evaluateJavascript will be the last write either
    // way, but this avoids briefly broadcasting pre-resume values.
    ViewCompat.requestApplyInsets(findViewById(android.R.id.content))
    pushInsetsToWeb()
  }

  override fun onDestroy() {
    // Cut the WebView reference before the activity tears down so any in-flight
    // postDelayed callbacks short-circuit in pushInsetsToWeb instead of calling
    // through to a detached WebView (and keeping this activity reachable).
    webView = null
    super.onDestroy()
  }

  private fun pushInsetsToWeb() {
    val wv = webView ?: return
    if (!hasReceivedInsets) return
    val js =
      "document.documentElement.style.setProperty('--android-inset-top', '${lastInsetTopCssPx}px');" +
        "document.documentElement.style.setProperty('--android-inset-bottom', '${lastInsetBottomCssPx}px');"
    wv.evaluateJavascript(js, null)
  }
}
