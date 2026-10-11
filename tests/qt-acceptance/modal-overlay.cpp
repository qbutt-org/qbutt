// SPDX-License-Identifier: GPL-2.0-or-later
#include "modal-overlay.h"

#include <functional>
#include <stdexcept>

#include <QAction>
#include <QApplication>
#include <QCloseEvent>
#include <QContextMenuEvent>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QListView>
#include <QMouseEvent>
#include <QPalette>
#include <QPushButton>
#include <QScrollBar>
#include <QShortcutEvent>
#include <QStringListModel>
#include <QThread>
#include <QToolButton>
#include <QVBoxLayout>

#include "base/global.h"
#include "gui/content/modaloverlay.h"

namespace
{
    class HostInputCounter final : public QObject
    {
    public:
        int keyPresses = 0;
        int keyReleases = 0;
        int contextMenus = 0;
        int closeRequests = 0;

    private:
        bool eventFilter(QObject *, QEvent *event) override
        {
            if (event->type() == QEvent::KeyPress)
                ++keyPresses;
            else if (event->type() == QEvent::KeyRelease)
                ++keyReleases;
            else if (event->type() == QEvent::ContextMenu)
                ++contextMenus;
            else if (event->type() == QEvent::Close)
                ++closeRequests;
            else
                return false;
            return true;
        }
    };

    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const QString &label, const std::function<bool()> &ready)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 5000))
        {
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
            QThread::msleep(1);
        }
        require(ready(), u"Timed out: %1"_s.arg(label));
    }

    void click(QWidget *widget, const QPoint &point)
    {
        const QPoint global = widget->mapToGlobal(point);
        QMouseEvent press(QEvent::MouseButtonPress, point, global,
            Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(widget, &press);
        QMouseEvent release(QEvent::MouseButtonRelease, point, global,
            Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
        QCoreApplication::sendEvent(widget, &release);
    }

    void key(QWidget *widget, const Qt::Key code, const Qt::KeyboardModifiers modifiers = Qt::NoModifier)
    {
        QKeyEvent press(QEvent::KeyPress, code, modifiers);
        QCoreApplication::sendEvent(widget, &press);
        QKeyEvent release(QEvent::KeyRelease, code, modifiers);
        QCoreApplication::sendEvent(widget, &release);
    }
}

void exerciseModalOverlay(QWidget *window, const QJsonObject &spec, QJsonObject &evidence)
{
    window->resize(1280, 720);
    window->show();
    window->activateWindow();
    waitFor(u"host activation"_s, [&] { return window->isActiveWindow(); });
    QPushButton background(u"Background action"_s, window);
    background.setGeometry(24, 48, 180, 32);
    background.show();
    background.setFocus();
    int backgroundActions = 0;
    QAction shortcut(window);
    shortcut.setShortcut(QKeySequence(u"Ctrl+J"_s));
    window->addAction(&shortcut);
    QObject::connect(&shortcut, &QAction::triggered, window, [&] { ++backgroundActions; });
    QShortcutEvent before(shortcut.shortcut(), nullptr);
    QCoreApplication::sendEvent(&shortcut, &before);
    require(backgroundActions == 1, u"Background shortcut fixture did not activate"_s);

    QImage checker(192, 192, QImage::Format_RGB32);
    for (int y = 0; y < checker.height(); ++y)
    {
        for (int x = 0; x < checker.width(); ++x)
            checker.setPixelColor(x, y, ((x / 8 + y / 8) % 2) ? Qt::black : Qt::white);
    }
    QLabel texture(window);
    texture.setGeometry(24, 112, 192, 192);
    texture.setPixmap(QPixmap::fromImage(checker));
    texture.show();

    Content::ModalOverlay overlay(window);
    overlay.setTitle(u"Chapters"_s);
    auto *layout = new QVBoxLayout(overlay.contentWidget());
    layout->setContentsMargins(0, 0, 0, 0);
    auto *search = new QLineEdit(overlay.contentWidget());
    search->setPlaceholderText(u"Search chapters"_s);
    layout->addWidget(search);
    auto *list = new QListView(overlay.contentWidget());
    list->setFrameShape(QFrame::NoFrame);
    layout->addWidget(list, 1);
    QStringList chapters;
    for (int chapter = 1; chapter <= 200; ++chapter)
        chapters.append(u"Volume %1, Chapter %2 — generated chapter title"_s.arg(chapter / 30 + 1).arg(chapter));
    QStringListModel model(chapters);
    list->setModel(&model);
    list->setCurrentIndex(model.index(120));
    int closures = 0;
    QObject::connect(&overlay, &Content::ModalOverlay::closed, window, [&] { ++closures; });
    overlay.open(search);
    waitFor(u"in-window modal layout"_s, [&]
    {
        return (QApplication::focusWidget() == search) && (list->verticalScrollBar()->maximum() > 0);
    });
    require(!overlay.isWindow() && (overlay.window() == window->window())
        && (overlay.geometry() == window->rect()), u"Modal created a separate window or missed its host bounds"_s);
    const auto screenshot = [&](const QString &name)
    {
        const QString path = spec.value(u"imageDir"_s).toString() + u"/"_s + name + u".png"_s;
        require(window->grab().save(path), u"Cannot save modal screenshot"_s);
        QJsonArray shots = evidence.value(u"screenshots"_s).toArray();
        shots.append(QJsonObject {{u"name"_s, name}, {u"path"_s, path},
            {u"width"_s, window->width()}, {u"height"_s, window->height()}});
        evidence[u"screenshots"_s] = shots;
    };
    const QImage dimmed = overlay.grab().toImage();
    int darkest = 255;
    int lightest = 0;
    for (int y = 144; y < 272; ++y)
    {
        for (int x = 56; x < 184; ++x)
        {
            const int value = dimmed.pixelColor(x, y).lightness();
            darkest = qMin(darkest, value);
            lightest = qMax(lightest, value);
        }
    }
    require((darkest > 0) && (lightest < 150) && (lightest - darkest < 100),
        u"Backdrop was not both blurred and dimmed"_s);
    list->verticalScrollBar()->setValue(list->verticalScrollBar()->maximum() / 2);
    const int scroll = list->verticalScrollBar()->value();
    search->setText(u"retained query"_s);
    screenshot(u"modal-1280-light"_s);
    click(search, search->rect().center());
    require(overlay.isVisible() && (closures == 0), u"Content click dismissed the modal"_s);
    key(search, Qt::Key_Escape, Qt::AltModifier);
    require(overlay.isVisible(), u"Modified Escape was consumed as modal dismissal"_s);

    for (int tab = 0; tab < 12; ++tab)
    {
        key(QApplication::focusWidget(), Qt::Key_Tab);
        require(overlay.isAncestorOf(QApplication::focusWidget()), u"Tab escaped into the background"_s);
    }
    QShortcutEvent blocked(shortcut.shortcut(), nullptr);
    QCoreApplication::sendEvent(&shortcut, &blocked);
    require(backgroundActions == 1, u"Modal allowed a background shortcut"_s);
    HostInputCounter hostInput;
    window->installEventFilter(&hostInput);
    auto *close = overlay.findChild<QToolButton *>(u"contentModalClose"_s);
    require(close && !close->icon().isNull() && !close->accessibleName().isEmpty(), u"Close action is missing"_s);
    key(close, Qt::Key_V, Qt::ControlModifier);
    key(list, Qt::Key_F6);
    QContextMenuEvent menu(QContextMenuEvent::Mouse, QPoint(8, 8), overlay.mapToGlobal(QPoint(8, 8)));
    QCoreApplication::sendEvent(&overlay, &menu);
    require((hostInput.keyPresses == 0) && (hostInput.keyReleases == 0) && (hostInput.contextMenus == 0),
        u"Unhandled keys or context menu reached the background"_s);
    QCloseEvent hostClose;
    QCoreApplication::sendEvent(window, &hostClose);
    require(hostInput.closeRequests == 1, u"Modal blocked the native window close path"_s);

    QWidget otherWindow;
    otherWindow.show();
    otherWindow.activateWindow();
    waitFor(u"other window activation"_s, [&]
    {
        return otherWindow.isActiveWindow() && !window->isActiveWindow();
    });
    key(&otherWindow, Qt::Key_Escape);
    require(overlay.isVisible() && (closures == 0), u"Window deactivation or external Escape dismissed the modal"_s);
    otherWindow.hide();
    window->activateWindow();
    waitFor(u"host reactivation"_s, [&] { return window->isActiveWindow(); });
    click(&overlay, QPoint(8, 8));
    require(!overlay.isVisible() && (closures == 1) && (QApplication::focusWidget() == &background),
        u"Backdrop close did not restore the previous focus"_s);
    overlay.open(search);
    QCoreApplication::processEvents();
    require((list->currentIndex() == model.index(120)) && (list->verticalScrollBar()->value() == scroll)
        && (search->text() == u"retained query"), u"Reopening reset selection, scroll or search"_s);
    click(close, close->rect().center());
    require(!overlay.isVisible() && (closures == 2), u"Close icon did not dismiss the modal"_s);
    overlay.open(search);
    key(search, Qt::Key_Escape);
    require(!overlay.isVisible() && (closures == 3), u"Escape did not dismiss the modal"_s);

    overlay.open(search);
    window->resize(1440, 900);
    QCoreApplication::processEvents();
    auto *panel = overlay.findChild<QWidget *>(u"contentModalPanel"_s);
    require(panel && (panel->size() == QSize(720, 600)) && overlay.rect().contains(panel->geometry())
        && panel->rect().contains(QRect(list->mapTo(panel, QPoint()), list->size())),
        u"Modal content exceeded the supported desktop layout"_s);
    screenshot(u"modal-1440-light"_s);
    const int beforeThemeScroll = list->verticalScrollBar()->value();
    const int lightBackdrop = overlay.grab().toImage().pixelColor(window->width() - 16, 400).lightness();
    const QPalette lightPalette = window->palette();
    QPalette darkPalette = lightPalette;
    darkPalette.setColor(QPalette::Window, QColor(32, 34, 37));
    darkPalette.setColor(QPalette::WindowText, Qt::white);
    darkPalette.setColor(QPalette::Base, QColor(24, 26, 29));
    darkPalette.setColor(QPalette::Text, Qt::white);
    darkPalette.setColor(QPalette::Button, QColor(42, 44, 47));
    darkPalette.setColor(QPalette::ButtonText, Qt::white);
    window->setPalette(darkPalette);
    waitFor(u"live dark backdrop"_s, [&]
    {
        return overlay.grab().toImage().pixelColor(window->width() - 16, 400).lightness() < lightBackdrop - 40;
    });
    require(overlay.isVisible() && (list->verticalScrollBar()->value() == beforeThemeScroll)
        && (list->currentIndex() == model.index(120)), u"Live appearance refresh reset the modal content"_s);
    const QImage closeIcon = close->icon().pixmap(close->iconSize()).toImage();
    bool readableClose = false;
    for (int y = 0; y < closeIcon.height(); ++y)
    {
        for (int x = 0; x < closeIcon.width(); ++x)
        {
            const QColor pixel = closeIcon.pixelColor(x, y);
            readableClose |= (pixel.alpha() > 200) && (pixel.lightness() > 200);
        }
    }
    require(readableClose, u"Close icon did not follow the dark palette"_s);
    screenshot(u"modal-1440-dark"_s);
    overlay.close();
    window->setPalette(lightPalette);
    QShortcutEvent after(shortcut.shortcut(), nullptr);
    QCoreApplication::sendEvent(&shortcut, &after);
    require(backgroundActions == 2, u"Closing did not release background shortcuts"_s);

    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    checks.append(QJsonObject {{u"name"_s, u"in-window-modal"_s}, {u"chapterCount"_s, model.rowCount()},
        {u"selectionAndScrollRetained"_s, true}, {u"deactivationRetained"_s, true},
        {u"backgroundShortcutBlocked"_s, true}, {u"backdropBlurContrast"_s, lightest - darkest},
        {u"liveAppearanceAndResize"_s, true},
        {u"rawInputBlockedNativeClosePreserved"_s, true},
        {u"backdropXEscape"_s, true}, {u"focusRestored"_s, true}});
    evidence[u"checks"_s] = checks;
}
