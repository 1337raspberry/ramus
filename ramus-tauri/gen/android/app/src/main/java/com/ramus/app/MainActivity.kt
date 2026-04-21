package com.ramus.app

import android.os.Bundle
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  private var webView: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

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
  }
}
