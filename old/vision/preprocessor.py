# cv2: color manipulation (BGR→grayscale), geometric transforms (resize), filtering (filter2D)
import cv2
import numpy as np



def to_grayscale(image: np.ndarray) -> np.ndarray:
    # cv2.cvtColor — color manipulation: convert BGR to grayscale
    return cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)

def upscale(image: np.ndarray, scale: float = 2.0) -> np.ndarray:
    # cv2.resize — geometric transformation: scale image for better OCR
    height, width = image.shape[:2]
    return cv2.resize(image, (int(width * scale), int(height * scale)), interpolation=cv2.INTER_CUBIC)

def sharpen(image: np.ndarray) -> np.ndarray:
    """Apply a sharpening kernel to make text edges crisper."""
    # cv2.filter2D — filtering/enhancement: apply custom convolution kernel to sharpen
    kernel = np.array([[0, -1, 0],
                       [-1, 5, -1],
                       [0, -1, 0]])
    return cv2.filter2D(image, -1, kernel)

def preprocess(image: np.ndarray, scale: float = 2.0) -> np.ndarray:
    """Full preprocessing pipeline: grayscale → upscale → sharpen."""
    image = to_grayscale(image)
    image = upscale(image, scale)
    image = sharpen(image)
    return image