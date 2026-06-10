package org.sunnypilot.dashdown

import java.net.Socket

/**
 * Minimal HTTP/1.1 POST to the `mock-copyparty` **control port** over a raw socket — used by the
 * background-sync live tests to inject server state at runtime (add a segment / add a drive).
 *
 * A raw socket sidesteps Android's cleartext-to-localhost block (`targetSdk≥28`) without adding a
 * `networkSecurityConfig` to the shipping app; the Rust core's own transfers go through reqwest and
 * are unaffected by Android's HTTP policy. Reach the host's control port from the device/emulator
 * via `adb reverse tcp:<port> tcp:<port>`.
 */
object MockControl {
  /** POST [json] to `http://127.0.0.1:[port][path]`, reading the response so the call completes. */
  fun post(port: Int, path: String, json: String) {
    Socket("127.0.0.1", port).use { sock ->
      val body = json.toByteArray(Charsets.UTF_8)
      val head =
          buildString {
                append("POST $path HTTP/1.1\r\n")
                append("Host: 127.0.0.1:$port\r\n")
                append("Content-Type: application/json\r\n")
                append("Content-Length: ${body.size}\r\n")
                append("Connection: close\r\n")
                append("\r\n")
              }
              .toByteArray(Charsets.US_ASCII)
      sock.getOutputStream().apply {
        write(head)
        write(body)
        flush()
      }
      sock.getInputStream().readBytes() // drain until close so the server finishes the request
    }
  }
}
