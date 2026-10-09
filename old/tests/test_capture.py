import numpy as np
from unittest.mock import patch, MagicMock
from capture.screen import capture_screen


def test_returns_numpy_array():
    # Create a fake BGRA image (height=100, width=200, 4 channels)
    fake_bgra = np.zeros((100, 200, 4), dtype=np.uint8)

    mock_sct = MagicMock()
    mock_sct.grab.return_value = fake_bgra

    with patch("mss.mss") as mock_mss:
        mock_mss.return_value.__enter__.return_value = mock_sct
        result = capture_screen(0, 0, 200, 100)

    assert isinstance(result, np.ndarray)


def test_drops_alpha_channel():
    fake_bgra = np.zeros((100, 200, 4), dtype=np.uint8)

    mock_sct = MagicMock()
    mock_sct.grab.return_value = fake_bgra

    with patch("mss.mss") as mock_mss:
        mock_mss.return_value.__enter__.return_value = mock_sct
        result = capture_screen(0, 0, 200, 100)

    assert result.shape[2] == 3  # must have 3 channels not 4