import numpy as np
import pytest
from vision.preprocessor import to_grayscale, upscale, sharpen, preprocess


def make_image(height=100, width=100, channels=3):
    """Helper to create a dummy BGR image for testing."""
    return np.random.randint(0, 255, (height, width, channels), dtype=np.uint8)


def test_to_grayscale_output_shape():
    image = make_image()
    result = to_grayscale(image)
    assert result.ndim == 2  # grayscale has no channel dimension


def test_upscale_default_doubles_size():
    image = make_image(100, 100)
    gray = to_grayscale(image)
    result = upscale(gray)
    assert result.shape == (200, 200)


def test_upscale_custom_scale():
    image = make_image(100, 100)
    gray = to_grayscale(image)
    result = upscale(gray, scale=3.0)
    assert result.shape == (300, 300)


def test_sharpen_preserves_shape():
    image = make_image(100, 100)
    gray = to_grayscale(image)
    result = sharpen(gray)
    assert result.shape == gray.shape


def test_preprocess_returns_2d():
    image = make_image()
    result = preprocess(image)
    assert result.ndim == 2


def test_preprocess_doubles_size_by_default():
    image = make_image(100, 100)
    result = preprocess(image)
    assert result.shape == (200, 200)