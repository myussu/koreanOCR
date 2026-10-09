from PySide6.QtWidgets import QApplication, QWidget, QRubberBand
from PySide6.QtCore import Qt, QRect, QPoint, QSize




class RegionPicker(QWidget):  # inherits everything QWidget has, we just customize it
    def __init__(self):
        super().__init__()  # runs QWidget's own setup before we add ours
        self.setWindowState(Qt.WindowState.WindowFullScreen)
        self.setWindowOpacity(0.3)  # transparent so user can see the game behind it
        self.setWindowFlags(Qt.WindowType.FramelessWindowHint | Qt.WindowType.WindowStaysOnTopHint)  # no title bar, always on top
        self.setCursor(Qt.CursorShape.CrossCursor)  # crosshair cursor signals "select something"
        self.rubberband = QRubberBand(QRubberBand.Shape.Rectangle, self)  # the dashed selection rectangle
        self.origin = QPoint()  # will store where the user first clicked
        self.region = None  # will store the final coordinates when done

    def mousePressEvent(self, event):  # Qt calls this automatically and passes event, which contains mouse position
        self.origin = event.position().toPoint()  # save the click position as our fixed starting corner
        self.rubberband.setGeometry(QRect(self.origin, QSize()))  # place rubber band at click point with zero size
        self.rubberband.show()

    def mouseMoveEvent(self, event):
        self.rubberband.setGeometry(QRect(self.origin, event.position().toPoint()).normalized())
        # QRect builds a rectangle from two points, normalized() fixes negative width/height when dragging left or up

    def mouseReleaseEvent(self, event):
        self.rubberband.hide()
        rect = QRect(self.origin, event.position().toPoint()).normalized()  # final rectangle from start to release point
        self.region = {
            "left": rect.x(),
            "top": rect.y(),
            "width": rect.width(),
            "height": rect.height()
        }  # stored as a dict because capture_screen expects exactly this format
        self.close()  # job done, close and free the window

def pick_region():
    app = QApplication.instance() or QApplication([])  # reuse existing app or create one, only one can exist at a time
    picker = RegionPicker()
    picker.show()  # creating a widget doesn't show it automatically, you must call this explicitly
    app.exec()  # starts the event loop, keeps the window alive and responsive until the user closes it
    return picker.region

if __name__ == "__main__":
    region = pick_region()
    print(region)

#If you have a 1440p monitor and scan the whole thing it should appear something like this {'left': 0, 'top': 0, 'width': 2560, 'height': 1440}