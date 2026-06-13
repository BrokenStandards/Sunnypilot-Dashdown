package org.sunnypilot.dashdown.ui.detail

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Pure-JVM round-trip tests for the HD-segment opaque URI codec. */
class HdMediaUriTest {

  @Test
  fun roundTripsAllFields() {
    val uri =
        HdMediaUri.build(
            deviceId = 7L, driveKey = "0000004f--3dbf7a8591--0", segNum = 42u, kindOrdinal = 1)
    val ref = HdMediaUri.parse(uri)!!
    assertEquals(7L, ref.deviceId)
    assertEquals("0000004f--3dbf7a8591--0", ref.driveKey)
    assertEquals(42, ref.segNum)
    assertEquals(1, ref.kindOrdinal)
  }

  @Test
  fun encodesSeparatorsInDriveKey() {
    // A drive key with characters that would break naive `/`-splitting must survive.
    val key = "weird|key/with--bits and spaces"
    val ref = HdMediaUri.parse(HdMediaUri.build(3L, key, 0u, 2))!!
    assertEquals(key, ref.driveKey)
    assertEquals(0, ref.segNum)
    assertEquals(2, ref.kindOrdinal)
  }

  @Test
  fun rejectsForeignOrMalformedUris() {
    assertNull(HdMediaUri.parse("file:///data/fcamera.hevc.mp4"))
    assertNull(HdMediaUri.parse("dashdownhd://hd/7/42")) // too few fields
    assertNull(HdMediaUri.parse("dashdownhd://hd/notanumber/0/0/key"))
  }
}
