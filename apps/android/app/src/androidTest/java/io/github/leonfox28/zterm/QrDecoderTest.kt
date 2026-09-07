package io.github.leonfox28.zterm

import android.graphics.BitmapFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.android.gms.tasks.Tasks
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class QrDecoderTest {
    @Test fun hostPngDecodesToExactCanonicalTicket() {
        val directory = InstrumentationRegistry.getInstrumentation().targetContext.filesDir
        val image = File(directory, "qr-fixture.png")
        val ticket = File(directory, "qr-fixture.txt")
        assumeTrue("explicit host-generated QR fixture", image.isFile && ticket.isFile)
        val bitmap = BitmapFactory.decodeFile(image.path)
        val scanner = BarcodeScanning.getClient(BarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build())
        try {
            val results = Tasks.await(scanner.process(InputImage.fromBitmap(bitmap, 0)), 10, TimeUnit.SECONDS)
            // Never print bearer contents as an assertion's expected/actual values.
            assertTrue("one exact canonical ticket", results.size == 1 && results.single().rawValue == ticket.readText().trim())
        } finally { scanner.close(); bitmap.recycle(); image.delete(); ticket.delete() }
    }
}
