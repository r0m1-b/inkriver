package io.github.r0m1_b.inkriver

import android.os.Bundle
import android.view.ViewGroup
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    ViewCompat.setOnApplyWindowInsetsListener(webView) { view, windowInsets ->
      val safeArea = windowInsets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(),
      )
      val layoutParams = view.layoutParams as? ViewGroup.MarginLayoutParams
      if (layoutParams != null && (
          layoutParams.leftMargin != safeArea.left ||
            layoutParams.topMargin != safeArea.top ||
            layoutParams.rightMargin != safeArea.right ||
            layoutParams.bottomMargin != safeArea.bottom
        )) {
        layoutParams.setMargins(
          safeArea.left,
          safeArea.top,
          safeArea.right,
          safeArea.bottom,
        )
        view.layoutParams = layoutParams
      }
      windowInsets
    }
    ViewCompat.requestApplyInsets(webView)
  }
}
