import cv2
import numpy as np


def frames_are_different(frame1: np.ndarray, frame2: np.ndarray, threshold: float = 0.02) -> bool:
    """
    Compare two grayscale frames and return True if they differ enough.
    threshold is the fraction of pixels that must have changed to count as different.
    """
    if frame1 is None or frame2 is None:
        return True

    if frame1.shape != frame2.shape:
        return True

    diff = cv2.absdiff(frame1, frame2)
    changed_pixels = np.count_nonzero(diff > 25)
    total_pixels = diff.size
    change_ratio = changed_pixels / total_pixels

    return change_ratio > threshold


def to_comparable(image: np.ndarray) -> np.ndarray:
    """Convert a BGR image to a small grayscale version for fast comparison."""
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
    return cv2.resize(gray, (320, 180))