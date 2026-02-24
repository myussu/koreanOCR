# cv2: reading images from file, display (native windows with imshow/waitKey)
import cv2
from preprocessor import preprocess

# cv2.imread — reading: load image from file
image = cv2.imread("img.png")
result = preprocess(image)

# cv2.imshow / waitKey / destroyAllWindows — display: show images in native windows
cv2.imshow("original", image)
cv2.imshow("preprocessed", result)
cv2.waitKey(0)
cv2.destroyAllWindows()