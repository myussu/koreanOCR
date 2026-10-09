import mss
import numpy as np
def capture_screen(left, top, width, height):
    with mss.mss() as sct:
        # The screen part to capture
        monitor = {"top": top, "left": left, "width": width, "height": height}
        # Grab the data
        sct_img = sct.grab(monitor)
        capture = np.array(sct_img) #Converting the capture into an array (mss format into np array) so we can send it ton opencv or any other ai model for ocr
        return capture[:, :, :3] #mss uses BGRA, the 4th value is transparency which we dont need, The :, : means "keep all rows and all columns", and :3 means "only keep the first 3 channels", dropping the alpha.