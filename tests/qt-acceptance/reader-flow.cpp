// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-flow.h"

#include <functional>
#include <stdexcept>

#include <QAbstractScrollArea>
#include <QAction>
#include <QApplication>
#include <QCheckBox>
#include <QColor>
#include <QComboBox>
#include <QContextMenuEvent>
#include <QCryptographicHash>
#include <QDialogButtonBox>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QFontInfo>
#include <QImage>
#include <QInputDialog>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QListView>
#include <QMap>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPushButton>
#include <QRawFont>
#include <QScrollArea>
#include <QScrollBar>
#include <QSortFilterProxyModel>
#include <QSpinBox>
#include <QTextBrowser>
#include <QTextDocument>
#include <QThread>
#include <QTimer>
#include <QToolButton>

#include "base/global.h"
#include "base/preferences.h"
#include "gui/content/readerwidget.h"
#include "gui/uithememanager.h"

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const QString &label, const std::function<bool()> &ready, const int timeout = 10000)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < timeout))
        {
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
            QThread::msleep(1);
        }
        require(ready(), u"Timed out: %1"_s.arg(label));
    }

    QJsonArray fixtureEvents(const QString &path)
    {
        QFile file(path);
        require(file.open(QIODevice::ReadOnly), u"Cannot read Reader fixture IPC log"_s);
        QJsonArray result;
        for (const QByteArray &line : file.readAll().split('\n'))
        {
            const QJsonDocument document = QJsonDocument::fromJson(line);
            if (document.isObject())
                result.append(document.object());
        }
        return result;
    }

    int countEvents(const QString &path, const QString &documentId, const QString &event, const QString &method = {})
    {
        int count = 0;
        for (const QJsonValue &value : fixtureEvents(path))
        {
            const QJsonObject entry = value.toObject();
            if ((entry.value(u"documentId"_s).toString() == documentId)
                && (entry.value(u"event"_s).toString() == event)
                && (method.isEmpty() || (entry.value(u"method"_s).toString() == method)))
            {
                ++count;
            }
        }
        return count;
    }

    void settle()
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (elapsed.elapsed() < 1300)
        {
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
            QThread::msleep(1);
        }
    }

    QByteArray fileHash(const QString &path)
    {
        QFile file(path);
        require(file.open(QIODevice::ReadOnly), u"Cannot hash generated Reader original"_s);
        return QCryptographicHash::hash(file.readAll(), QCryptographicHash::Sha256);
    }

    void clickCanvas(QWidget *widget, const QPoint &point)
    {
        QMouseEvent press(QEvent::MouseButtonPress, point, widget->mapToGlobal(point),
            Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(widget, &press);
        QMouseEvent release(QEvent::MouseButtonRelease, point, widget->mapToGlobal(point),
            Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
        QCoreApplication::sendEvent(widget, &release);
    }

    void pressKey(QWidget *widget, const int key)
    {
        QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier);
        QCoreApplication::sendEvent(widget, &press);
        QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier);
        QCoreApplication::sendEvent(widget, &release);
    }

    void exerciseReaderSettings(const QJsonObject &spec, QJsonObject &evidence)
    {
        const bool persistenceProbe = spec.value(u"readerSettingsProbe"_s).toBool();
        const QString logPath = spec.value(u"fixtureLog"_s).toString();
        const QString screenshotDir = spec.value(u"imageDir"_s).toString();
        const Content::ReaderSettings persisted {
            .mode = Content::ReaderMode::Continuous,
            .imageFit = Content::ImageFit::Width,
            .enlargeSmallImages = false,
            .brightness = 75,
            .pageGap = 18,
            .contentWidth = 80
        };
        require(Preferences::instance()->readerSettings() == (persistenceProbe ? persisted : Content::ReaderSettings()),
            persistenceProbe ? u"Reader settings did not survive a fresh process"_s : u"Fresh Reader defaults changed"_s);

        Content::ReaderWidget reader;
        reader.resize(900, 700);
        reader.show();
        auto *images = reader.findChild<QAbstractScrollArea *>(u"readerImages"_s);
        auto *gear = reader.findChild<QToolButton *>(u"readerSettings"_s);
        auto *previous = reader.findChild<QToolButton *>(u"readerPrevious"_s);
        auto *next = reader.findChild<QToolButton *>(u"readerNext"_s);
        auto *translate = reader.findChild<QPushButton *>(u"readerTranslate"_s);
        auto *original = reader.findChild<QPushButton *>(u"readerOriginal"_s);
        auto *pageNumber = reader.findChild<QLabel *>(u"readerPageNumber"_s);
        require(images && gear && previous && next && translate && original && pageNumber,
            u"Reader settings scenario controls are missing"_s);
        QWidget *canvas = images->viewport();
        QScrollBar *bar = images->verticalScrollBar();
        const auto recordImageGeometry = [&](const QString &stage)
        {
            const QRect rect = images->property("imageRect").toRect();
            const QJsonObject geometry {{u"event"_s, u"reader-geometry"_s}, {u"stage"_s, stage},
                {u"documentId"_s, u"settings"_s}, {u"value"_s, bar->value()},
                {u"maximum"_s, bar->maximum()}, {u"viewportHeight"_s, canvas->height()},
                {u"imageIndex"_s, images->property("imageIndex").toInt()},
                {u"imageTop"_s, rect.top()}, {u"imageHeight"_s, rect.height()},
                {u"offset"_s, double(8 - rect.top()) / rect.height()}};
            QFile file(logPath);
            require(file.open(QIODevice::WriteOnly | QIODevice::Append), u"Cannot record Reader image geometry"_s);
            file.write(QJsonDocument(geometry).toJson(QJsonDocument::Compact) + '\n');
        };
        QJsonArray checks = evidence.value(u"checks"_s).toArray();
        QJsonArray screenshots = evidence.value(u"screenshots"_s).toArray();
        const auto open = [&](const QString &documentId)
        {
            const int before = countEvents(logPath, documentId, u"original"_s);
            reader.open({{u"work"_s, QJsonObject {{u"id"_s, documentId}, {u"title"_s, documentId}}}});
            waitFor(u"settings fixture original"_s, [&]
            {
                return (countEvents(logPath, documentId, u"original"_s) > before) && translate->isEnabled();
            });
        };
        const auto waitForIndex = [&](const int index)
        {
            waitFor(u"settings page %1"_s.arg(index + 1), [&]
            {
                return translate->isEnabled() && (images->property("imageIndex").toInt() == index);
            });
        };
        const auto screenshot = [&](QWidget *widget, const QString &name)
        {
            const QString path = screenshotDir + u"/"_s + name + u".png"_s;
            require(widget->grab().save(path), u"Cannot save Reader settings screenshot"_s);
            screenshots.append(QJsonObject {{u"name"_s, name}, {u"path"_s, path},
                {u"width"_s, widget->width()}, {u"height"_s, widget->height()}});
            evidence[u"screenshots"_s] = screenshots;
        };

        QDialog *dialog = nullptr;
        QComboBox *mode = nullptr;
        QComboBox *fit = nullptr;
        QCheckBox *enlarge = nullptr;
        QSpinBox *brightness = nullptr;
        QSpinBox *gap = nullptr;
        QSpinBox *width = nullptr;
        const auto openDialog = [&]
        {
            gear->click();
            waitFor(u"Reader settings dialog"_s, [&]
            {
                dialog = reader.findChild<QDialog *>(u"readerSettingsDialog"_s);
                return dialog && dialog->isVisible();
            });
            mode = dialog->findChild<QComboBox *>(u"readerMode"_s);
            fit = dialog->findChild<QComboBox *>(u"readerImageFit"_s);
            enlarge = dialog->findChild<QCheckBox *>(u"readerEnlargeSmallImages"_s);
            brightness = dialog->findChild<QSpinBox *>(u"readerBrightness"_s);
            gap = dialog->findChild<QSpinBox *>(u"readerPageGap"_s);
            width = dialog->findChild<QSpinBox *>(u"readerContentWidth"_s);
            require(mode && fit && enlarge && brightness && gap && width, u"Reader settings controls are missing"_s);
            for (QWidget *control : QList<QWidget *> {mode, fit, enlarge, brightness, gap, width})
                require(control->isVisible(), u"A Reader setting is hidden instead of remaining on its page"_s);
        };
        const auto requireSavedControls = [&]
        {
            require((mode->currentIndex() == 1) && (fit->currentIndex() == 1) && !enlarge->isChecked()
                && (brightness->value() == 75) && (gap->value() == 18) && (width->value() == 80),
                u"Reader settings dialog did not load the saved values"_s);
        };

        open(persistenceProbe ? u"settings-probe"_s : u"settings"_s);
        waitForIndex(0);
        openDialog();
        if (persistenceProbe)
        {
            requireSavedControls();
            const QRect rect = images->property("imageRect").toRect();
            require((rect.width() < canvas->width() * 0.9) && (bar->maximum() > 0),
                u"A fresh Reader ignored the saved continuous mode or content width"_s);
            checks.append(u"A fresh process and Reader widget restore every persisted Reader setting and apply it to images"_s);
            evidence[u"checks"_s] = checks;
            return;
        }

        require((mode->currentIndex() == 0) && (fit->currentIndex() == 0) && enlarge->isChecked()
            && (brightness->value() == 100) && (gap->value() == 0) && (width->value() == 100),
            u"The Reader settings dialog does not reflect the product defaults"_s);
        require(!gap->isEnabled(), u"Page gap is actionable outside continuous reading"_s);
        next->click();
        next->click();
        waitForIndex(2);
        require(canvas->rect().contains(images->property("imageRect").toRect()),
            u"Default screen-height fitting clips a tall page"_s);
        fit->setCurrentIndex(1);
        waitFor(u"width-fitted tall image can scroll"_s, [&]
        {
            return bar->isVisible() && (bar->maximum() > canvas->height());
        });
        require(images->property("imageRect").toRect().height() > canvas->height(),
            u"Width fitting did not expose the tall image at readable scale"_s);
        bar->setValue(bar->maximum() / 3);
        const int fullWidth = images->property("imageRect").toRect().width();
        width->setValue(80);
        QCoreApplication::processEvents();
        const int reducedWidth = images->property("imageRect").toRect().width();
        require((reducedWidth < fullWidth * 0.9) && (reducedWidth > fullWidth * 0.7) && (bar->value() > 0),
            u"Content width did not resize the image while retaining its reading position"_s);
        waitForIndex(2);
        mode->setCurrentIndex(1);
        waitForIndex(2);
        require(gap->isEnabled() && (bar->value() > 0), u"Continuous mode lost the selected page or its scroll position"_s);
        fit->setCurrentIndex(0);
        width->setValue(100);
        previous->click();
        waitForIndex(1);
        const int touchingPages = bar->maximum();
        gap->setValue(24);
        waitFor(u"continuous page spacing"_s, [&] { return bar->maximum() > touchingPages; });
        waitForIndex(1);
        const int middlePagePosition = bar->value();
        require(middlePagePosition > 0, u"Continuous navigation did not reveal the middle page"_s);
        bar->setValue(middlePagePosition - 8);
        waitForIndex(1);
        const int gapPosition = bar->value();
        brightness->setValue(99);
        require(bar->value() == gapPosition, u"Changing brightness moved the reading position inside a page gap"_s);
        brightness->setValue(100);
        bar->setValue(middlePagePosition);
        pressKey(dialog, Qt::Key_Escape);
        require(!dialog->isVisible(), u"Escape did not close Reader settings"_s);
        reader.activateWindow();
        waitFor(u"active continuous Reader"_s, [&] { return reader.isActiveWindow(); });
        bar->setValue(0);
        waitForIndex(0);
        bar->setValue(middlePagePosition);
        waitForIndex(1);
        require(pageNumber->text() == u"2 / 3", u"Continuous scrolling did not update the page indicator"_s);
        clickCanvas(canvas, canvas->rect().center());
        waitForIndex(1);
        settle();
        translate->click();
        waitFor(u"continuous translation starts at the visible image"_s, [&]
        {
            return (countEvents(logPath, u"settings"_s, u"held"_s) == 1) && original->isVisible();
        });
        bool translatedMiddlePage = false;
        bool savedMiddlePage = false;
        for (const QJsonValue &value : fixtureEvents(logPath))
        {
            const QJsonObject entry = value.toObject();
            if (entry.value(u"documentId"_s).toString() != u"settings")
                continue;
            const bool middlePage = entry.value(u"params"_s).toObject().value(u"imageIndex"_s).toInt(-1) == 1;
            if (entry.value(u"method"_s).toString() == u"reader.ahead")
                translatedMiddlePage = middlePage;
            if ((entry.value(u"method"_s).toString() == u"reader.position") && middlePage)
                savedMiddlePage = true;
        }
        require(translatedMiddlePage, u"Continuous mode translated a page other than the visible page"_s);
        require(savedMiddlePage, u"Continuous scrolling did not persist the visible page"_s);
        const int translatedPosition = bar->value();
        original->click();
        waitForIndex(1);
        require(bar->value() == translatedPosition, u"Original moved the continuous reading position"_s);
        original->click();
        waitForIndex(1);
        require(bar->value() == translatedPosition, u"Translation moved the continuous reading position"_s);
        translate->click();
        checks.append(u"Live height/width fit, content width and continuous spacing affect image geometry and retain the current page"_s);
        checks.append(u"Continuous scrolling updates the page indicator, persistence and AI index; canvas clicks and Original keep the page"_s);

        openDialog();
        fit->setCurrentIndex(1);
        pressKey(dialog, Qt::Key_Escape);
        next->click();
        waitForIndex(2);
        require(bar->maximum() - bar->value() > canvas->height() / 3,
            u"The continuous fixture must leave room to scroll inside its tall image"_s);
        bar->setValue(bar->value() + (canvas->height() / 3));
        const int continuousPosition = bar->value();
        recordImageGeometry(u"continuous-chosen"_s);
        settle();
        recordImageGeometry(u"continuous-settled"_s);
        open(u"settings"_s);
        waitForIndex(2);
        recordImageGeometry(u"continuous-reopened"_s);
        require(qAbs(bar->value() - continuousPosition) <= 2,
            u"Reopening continuous manga lost the position inside its current image"_s);
        openDialog();
        mode->setCurrentIndex(0);
        fit->setCurrentIndex(1);
        pressKey(dialog, Qt::Key_Escape);
        bar->setValue(bar->maximum() / 3);
        const int pagedPosition = bar->value();
        require(pagedPosition > 0, u"The tall paged image has no position to restore"_s);
        settle();
        open(u"settings"_s);
        waitForIndex(2);
        require(qAbs(bar->value() - pagedPosition) <= 2,
            u"Reopening width-fitted manga lost its position inside the current page"_s);
        checks.append(u"Reopening continuous and width-fitted paged manga restores the current image and its internal scroll offset"_s);

        previous->click();
        waitForIndex(1);
        open(u"small-image"_s);
        waitForIndex(0);
        const int enlargedWidth = images->property("imageRect").toRect().width();
        require(enlargedWidth > 160, u"The default does not enlarge small images"_s);
        openDialog();
        enlarge->setChecked(false);
        QCoreApplication::processEvents();
        const QRect smallRect = images->property("imageRect").toRect();
        require((smallRect.width() <= 160) && (smallRect.height() <= 120) && (smallRect.width() < enlargedWidth),
            u"Disabling enlargement did not preserve the small image's native dimensions"_s);
        const QPoint sample(smallRect.x() + (smallRect.width() / 10), smallRect.center().y());
        const int fullBrightness = canvas->grab().toImage().pixelColor(sample).lightness();
        const QColor chrome = gear->palette().button().color();
        brightness->setValue(75);
        const int dimmedBrightness = canvas->grab().toImage().pixelColor(sample).lightness();
        require((fullBrightness - dimmedBrightness > 30) && (gear->palette().button().color() == chrome),
            u"Brightness did not dim the image independently of Reader controls"_s);
        mode->setCurrentIndex(1);
        fit->setCurrentIndex(1);
        gap->setValue(18);
        width->setValue(80);
        const auto saved = Preferences::instance()->readerSettings();
        require(saved == persisted,
            u"Live Reader settings differ (actual / expected): mode %1 / %2, fit %3 / %4, enlarge %5 / %6, brightness %7 / %8, gap %9 / %10, width %11 / %12"_s
                .arg(static_cast<int>(saved.mode)).arg(static_cast<int>(persisted.mode))
                .arg(static_cast<int>(saved.imageFit)).arg(static_cast<int>(persisted.imageFit))
                .arg(saved.enlargeSmallImages).arg(persisted.enlargeSmallImages)
                .arg(saved.brightness).arg(persisted.brightness)
                .arg(saved.pageGap).arg(persisted.pageGap)
                .arg(saved.contentWidth).arg(persisted.contentWidth));
        auto *buttons = dialog->findChild<QDialogButtonBox *>();
        require(buttons && buttons->button(QDialogButtonBox::Close), u"Reader settings has no Close action"_s);
        buttons->button(QDialogButtonBox::Close)->click();
        require(!dialog->isVisible(), u"Close did not dismiss Reader settings"_s);
        open(u"settings"_s);
        waitForIndex(1);
        openDialog();
        requireSavedControls();
        for (const auto scheme : {ColorScheme::Light, ColorScheme::Dark})
        {
            UIThemeManager::instance()->previewColorScheme(scheme);
            QCoreApplication::processEvents();
            const QString theme = scheme == ColorScheme::Light ? u"light"_s : u"dark"_s;
            dialog->resize(420, 340);
            QCoreApplication::processEvents();
            screenshot(&reader, u"continuous-reader-"_s + theme);
            screenshot(dialog, u"reader-settings-"_s + theme);
            dialog->resize(dialog->minimumSize());
            QCoreApplication::processEvents();
            auto *scroll = dialog->findChild<QScrollArea *>();
            require(scroll && (scroll->verticalScrollBar()->maximum() > 0), u"Compact Reader settings cannot scroll to all controls"_s);
            scroll->ensureWidgetVisible(width);
            const QRect widthRect(width->mapTo(scroll->viewport(), QPoint()), width->size());
            require(scroll->viewport()->rect().intersects(widthRect), u"Content width is unreachable in compact Reader settings"_s);
            screenshot(dialog, u"reader-settings-compact-"_s + theme);
        }
        UIThemeManager::instance()->previewColorScheme(ColorScheme::Light);
        pressKey(dialog, Qt::Key_Escape);
        require(!dialog->isVisible() && (Preferences::instance()->readerSettings() == persisted),
            u"Closing Reader settings discarded live changes"_s);
        checks.append(u"Small-image enlargement and image-only brightness work; all settings remain reachable with compact scrolling in Light and Dark"_s);
        checks.append(u"Close and Escape retain all live settings; reopening the dialog restores their values"_s);
        evidence[u"checks"_s] = checks;
        evidence[u"fixtureEvents"_s] = fixtureEvents(logPath);
    }
}

void exerciseReaderFlow(MainWindow *, const QJsonObject &spec, QJsonObject &evidence)
{
    if (spec.value(u"readerSettingsProbe"_s).toBool())
    {
        exerciseReaderSettings(spec, evidence);
        return;
    }
    const QString fixtureImageDir = spec.value(u"fixtureImages"_s).toString();
    const QString screenshotDir = spec.value(u"imageDir"_s).toString();
    const QString logPath = spec.value(u"fixtureLog"_s).toString();
    const QMap<QString, QSize> fixtureSizes {{u"portrait"_s, QSize(720, 960)},
        {u"landscape"_s, QSize(960, 540)}, {u"tall"_s, QSize(480, 1200)}, {u"small"_s, QSize(160, 120)}};
    QMap<QString, QByteArray> originalHashes;
    for (auto it = fixtureSizes.cbegin(); it != fixtureSizes.cend(); ++it)
    {
        const QSize size = it.value();
        QImage original(size, QImage::Format_RGB32);
        original.fill(Qt::white);
        {
            QPainter painter(&original);
            painter.fillRect(QRect(0, 0, size.width(), 24), QColor(0, 157, 247));
            painter.setPen(Qt::black);
            QFont font = painter.font();
            font.setPixelSize(30);
            painter.setFont(font);
            painter.drawText(QRect(size.width() / 4, size.height() / 5, size.width() / 2, size.height() * 3 / 10),
                Qt::AlignCenter, u"Original source text"_s);
            painter.drawText(QRect(0, size.height() * 3 / 4, size.width(), size.height() / 5),
                Qt::AlignCenter, it.key());
        }
        const QString originalPath = fixtureImageDir + u"/"_s + it.key() + u".png"_s;
        require(original.save(originalPath), u"Cannot save generated Reader original"_s);
        originalHashes.insert(originalPath, fileHash(originalPath));
        QImage cleaned = original;
        {
            QPainter painter(&cleaned);
            painter.fillRect(QRect(size.width() / 5, size.height() * 3 / 20, size.width() * 3 / 5, size.height() * 2 / 5),
                Qt::white);
        }
        require(cleaned.save(fixtureImageDir + u"/cleaned-"_s + it.key() + u".png"_s),
            u"Cannot save generated Reader clean layer"_s);
    }

    Content::ReaderWidget reader;
    reader.resize(700, 580);
    reader.show();
    reader.activateWindow();
    auto *translate = reader.findChild<QPushButton *>(u"readerTranslate"_s);
    auto *showOriginal = reader.findChild<QPushButton *>(u"readerOriginal"_s);
    auto *images = reader.findChild<QAbstractScrollArea *>(u"readerImages"_s);
    auto *previous = reader.findChild<QToolButton *>(u"readerPrevious"_s);
    auto *next = reader.findChild<QToolButton *>(u"readerNext"_s);
    auto *chapters = reader.findChild<QComboBox *>(u"readerChapters"_s);
    auto *pageNumber = reader.findChild<QLabel *>(u"readerPageNumber"_s);
    auto *text = reader.findChild<QTextBrowser *>(u"readerText"_s);
    auto *textSize = reader.findChild<QSpinBox *>(u"readerTextSize"_s);
    auto *back = reader.findChild<QToolButton *>(u"readerBack"_s);
    auto *title = reader.findChild<QLabel *>(u"readerTitle"_s);
    require(translate && showOriginal && images && previous && next && chapters && pageNumber && text && textSize && back && title,
        u"Native Reader controls are missing"_s);
    require(QFontInfo(images->font()).family() == u"Neucha", u"Manga lettering fell back to the interface font"_s);
    QWidget *canvas = images->viewport();
    const QRawFont mangaFont = QRawFont::fromFont(images->font());
    const QString russianAlphabet = u"АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя«»—…!?"_s;
    for (const QChar letter : russianAlphabet)
        require(mangaFont.supportsCharacter(letter), u"The bundled manga font is missing %1"_s.arg(letter));
    require(QFontInfo(text->document()->defaultFont()).family() != u"Neucha", u"Manga lettering leaked into text chapters"_s);
    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    QJsonArray screenshots;

    const auto open = [&](const QString &documentId)
    {
        const int originals = countEvents(logPath, documentId, u"original"_s);
        reader.open({{u"work"_s, QJsonObject {{u"id"_s, documentId}, {u"title"_s, documentId}}}});
        waitFor(u"cached original %1"_s.arg(documentId), [&]
        {
            return (countEvents(logPath, documentId, u"original"_s) > originals) && translate->isEnabled();
        });
    };
    const auto waitForPage = [&](const int chapter, const int index, const int count)
    {
        waitFor(u"chapter %1, page %2"_s.arg(chapter + 1).arg(index + 1), [&]
        {
            return translate->isEnabled() && (chapters->currentIndex() == chapter)
                && (images->property("imageIndex").toInt() == index)
                && (pageNumber->text() == u"%1 / %2"_s.arg(index + 1).arg(count));
        });
    };
    const auto requireFit = [&](const QSize &source)
    {
        const QRect rect = images->property("imageRect").toRect();
        require(rect.isValid() && canvas->rect().contains(rect), u"The full manga page does not fit the canvas"_s);
        require(qAbs((rect.width() * source.height()) - (rect.height() * source.width())) <= qMax(source.width(), source.height()),
            u"The manga page aspect ratio was distorted"_s);
    };
    const auto saveScreenshot = [&](const QString &name)
    {
        const QString path = screenshotDir + u"/"_s + name + u".png"_s;
        require(reader.grab().save(path), u"Cannot save Reader acceptance screenshot"_s);
        screenshots.append(QJsonObject {{u"name"_s, name}, {u"path"_s, path},
            {u"imageIndex"_s, images->property("imageIndex").toInt()},
            {u"width"_s, reader.width()}, {u"height"_s, reader.height()}});
        evidence[u"screenshots"_s] = screenshots;
    };

    {
        reader.resize(1280, 720);
        open(u"volumes"_s);
        require((chapters->count() == 121) && (chapters->itemText(0) == u"Volume 1 — Prologue")
            && (chapters->itemText(20) == u"Volume 2 — Prologue") && (chapters->itemText(120) == u"Prologue"),
            u"Chapter labels lost known volumes or invented an unknown volume"_s);
        chapters->setFocus();
        clickCanvas(chapters, chapters->rect().center());
        auto *overlay = reader.findChild<QWidget *>(u"readerChapterOverlay"_s);
        auto *search = reader.findChild<QLineEdit *>(u"readerChapterSearch"_s);
        auto *list = reader.findChild<QListView *>(u"readerChapterList"_s);
        require(overlay && search && list, u"The chapter picker controls are missing"_s);
        auto *filter = qobject_cast<QSortFilterProxyModel *>(list->model());
        require(filter && (filter->sourceModel() == chapters->model()), u"The chapter picker lost the source model"_s);
        waitFor(u"chapter picker focus"_s, [&] { return overlay->isVisible() && search->hasFocus(); });
        require(overlay->window() == reader.window(), u"Chapter selection opened outside the Reader window"_s);

        const int reads = countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s);
        QLabel *empty = nullptr;
        for (QLabel *label : overlay->findChildren<QLabel *>())
        {
            if (label->text() == u"No matching chapters")
                empty = label;
        }
        require(empty, u"The chapter picker has no empty-search message"_s);
        search->setText(u"A chapter that does not exist"_s);
        waitFor(u"empty chapter search"_s, [&] { return (filter->rowCount() == 0) && empty->isVisible() && !list->isVisible(); });
        saveScreenshot(u"chapters-empty"_s);
        search->clear();
        waitFor(u"chapter search recovery"_s, [&] { return (filter->rowCount() == 121) && list->isVisible() && !empty->isVisible(); });
        for (int row = 0; row < filter->rowCount(); ++row)
        {
            require(filter->index(row, 0).data(Qt::UserRole).toString() == u"volumes:%1"_s.arg(row),
                u"Clearing an empty search changed source identities or order"_s);
        }
        search->setText(u"Volume 2"_s);
        require(filter->rowCount() == 20, u"Volume search did not find its twenty chapters"_s);
        for (int row = 0; row < filter->rowCount(); ++row)
        {
            require(filter->index(row, 0).data(Qt::UserRole).toString() == u"volumes:%1"_s.arg(row + 20),
                u"Volume search included another volume's chapter"_s);
        }
        search->setText(u"prologue"_s);
        require(filter->rowCount() == 7, u"Case-insensitive title search lost repeated prologues"_s);
        for (int row = 0; row < filter->rowCount(); ++row)
        {
            require(filter->index(row, 0).data(Qt::UserRole).toString() == u"volumes:%1"_s.arg(row * 20),
                u"Title search changed source identities or order"_s);
        }
        QCoreApplication::processEvents();
        require((chapters->currentIndex() == 0) && (countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s) == reads),
            u"Typing a chapter filter opened a chapter"_s);
        const QModelIndex secondPrologue = filter->index(1, 0);
        clickCanvas(list->viewport(), list->visualRect(secondPrologue).center());
        const auto waitForChapter = [&](const int index)
        {
            const QString id = u"volumes:%1"_s.arg(index);
            waitFor(u"exact chapter %1"_s.arg(id), [&]
            {
                return translate->isEnabled() && (chapters->currentIndex() == index)
                    && (chapters->currentData().toString() == id) && text->toPlainText().contains(u"Original text of %1."_s.arg(id));
            });
        };
        waitForChapter(20);
        settle();
        require(!overlay->isVisible() && (countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s) == reads + 1),
            u"Single-click chapter activation did not close once and read exactly once"_s);
        QJsonObject latestChapter;
        for (const QJsonValue &value : fixtureEvents(logPath))
        {
            const QJsonObject entry = value.toObject();
            if ((entry.value(u"documentId"_s).toString() == u"volumes") && (entry.value(u"method"_s).toString() == u"reader.chapter"))
                latestChapter = entry.value(u"params"_s).toObject().value(u"chapter"_s).toObject();
        }
        require(latestChapter.value(u"id"_s).toString() == u"volumes:20", u"Filtered row index escaped into the source chapter request"_s);

        clickCanvas(chapters, chapters->rect().center());
        waitFor(u"retained chapter query"_s, [&] { return overlay->isVisible() && search->hasFocus(); });
        require((search->text() == u"prologue") && (filter->rowCount() == 7)
            && (list->currentIndex().data(Qt::UserRole).toString() == u"volumes:20"), u"Reopening reset the chapter query or selection"_s);
        search->setText(u"Volume 2"_s);
        list->setFocus();
        pressKey(QApplication::focusWidget(), Qt::Key_Down);
        pressKey(QApplication::focusWidget(), Qt::Key_Return);
        waitForChapter(21);
        settle();
        require(!overlay->isVisible() && (countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s) == reads + 2),
            u"Enter did not activate exactly one filtered chapter"_s);

        clickCanvas(chapters, chapters->rect().center());
        waitFor(u"chapter picker reopened"_s, [&] { return overlay->isVisible() && search->hasFocus(); });
        list->setFocus();
        pressKey(QApplication::focusWidget(), Qt::Key_Down);
        const QVariant retainedRow = list->currentIndex().data(Qt::UserRole);
        require(retainedRow.toString() == u"volumes:22", u"The picker did not highlight an unread chapter"_s);
        QCoreApplication::processEvents();
        QScrollBar *chapterScroll = list->verticalScrollBar();
        chapterScroll->setValue(chapterScroll->maximum() / 2);
        const int retainedScroll = chapterScroll->value();
        require(retainedScroll > 0, u"The generated chapter list did not establish a nonzero scroll position"_s);
        for (const int delta : {0, -1, 1})
        {
            require(overlay->isAncestorOf(QApplication::focusWidget()), u"Chapter picker focus escaped before Escape"_s);
            pressKey(QApplication::focusWidget(), Qt::Key_Escape);
            waitFor(u"chapter picker focus restored"_s, [&] { return !overlay->isVisible() && chapters->hasFocus(); });
            if (delta != 0)
            {
                (delta < 0 ? previous : next)->click();
                waitForChapter(delta < 0 ? 20 : 21);
            }
            clickCanvas(chapters, chapters->rect().center());
            waitFor(u"chapter picker reopened after navigation"_s, [&] { return overlay->isVisible() && search->hasFocus(); });
            settle();
            require((search->text() == u"Volume 2") && (filter->rowCount() == 20)
                && (list->currentIndex().data(Qt::UserRole) == (delta == 0 ? retainedRow : chapters->currentData()))
                && (chapterScroll->value() == retainedScroll),
                u"Closing or chapter navigation reset picker state: delta=%1 query=%2 rows=%3 current=%4 scroll=%5 expected=%6"_s
                    .arg(delta).arg(search->text()).arg(filter->rowCount()).arg(list->currentIndex().data(Qt::UserRole).toString())
                    .arg(chapterScroll->value()).arg(retainedScroll));
            if (delta == 0)
            {
                require((chapters->currentIndex() == 21) && (countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s) == reads + 2),
                    u"Browsing the chapter list opened an unactivated chapter"_s);
            }
        }
        const QPoint backdrop = reader.mapToGlobal(QPoint(8, reader.height() - 8));
        QWidget *receiver = QApplication::widgetAt(backdrop);
        require(receiver && ((receiver == overlay) || overlay->isAncestorOf(receiver)), u"The modal backdrop does not intercept Reader input"_s);
        const int readsBeforeBackdrop = countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s);
        clickCanvas(receiver, receiver->mapFromGlobal(backdrop));
        waitFor(u"chapter picker backdrop close"_s, [&] { return !overlay->isVisible(); });
        clickCanvas(chapters, chapters->rect().center());
        waitFor(u"chapter picker reopened after backdrop"_s, [&] { return overlay->isVisible() && search->hasFocus(); });
        settle();
        require((search->text() == u"Volume 2") && (chapterScroll->value() == retainedScroll)
            && (list->currentIndex().data(Qt::UserRole).toString() == u"volumes:21"), u"Backdrop close discarded the chapter picker state"_s);
        require((chapters->currentData().toString() == u"volumes:21")
            && (countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s) == readsBeforeBackdrop),
            u"Backdrop close activated a chapter"_s);

        search->clear();
        for (const auto scheme : {ColorScheme::Light, ColorScheme::Dark})
        {
            UIThemeManager::instance()->previewColorScheme(scheme);
            for (const QSize size : {QSize(1280, 720), QSize(1440, 900)})
            {
                reader.resize(size);
                QCoreApplication::processEvents();
                const QModelIndex longChapter = filter->index(21, 0);
                const QString fullTitle = chapters->itemText(21);
                list->scrollTo(longChapter, QAbstractItemView::PositionAtCenter);
                require((overlay->geometry() == reader.rect()) && reader.rect().contains(QRect(list->mapTo(&reader, QPoint()), list->size())),
                    u"The chapter overlay or list does not fit the Reader window"_s);
                require((fullTitle.size() > 180) && (list->fontMetrics().horizontalAdvance(fullTitle) > list->viewport()->width())
                    && (longChapter.data(Qt::ToolTipRole).toString() == fullTitle), u"The elided chapter title is not fully available in its tooltip"_s);
                saveScreenshot(u"chapters-%1-%2"_s.arg(scheme == ColorScheme::Light ? u"light"_s : u"dark"_s).arg(size.width()));
            }
        }
        pressKey(QApplication::focusWidget(), Qt::Key_Escape);
        waitFor(u"chapter picker final close"_s, [&] { return !overlay->isVisible(); });
        open(u"volumes"_s);
        waitForChapter(21);
        const int readsAfterReopen = countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s);
        clickCanvas(chapters, chapters->rect().center());
        overlay = reader.findChild<QWidget *>(u"readerChapterOverlay"_s);
        search = reader.findChild<QLineEdit *>(u"readerChapterSearch"_s);
        list = reader.findChild<QListView *>(u"readerChapterList"_s);
        require(overlay && search && list, u"The chapter picker was not recreated after document reload"_s);
        filter = qobject_cast<QSortFilterProxyModel *>(list->model());
        require(filter, u"The reloaded chapter picker has no filter model"_s);
        waitFor(u"chapter picker after document reload"_s, [&] { return overlay->isVisible() && search->hasFocus(); });
        settle();
        require(search->text().isEmpty() && (list->currentIndex().data(Qt::UserRole).toString() == u"volumes:21")
            && (countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s) == readsAfterReopen),
            u"Reloading the document lost its chapter highlight or opened another chapter"_s);
        const QString originalText = text->toPlainText();
        auto *status = reader.findChild<QLabel *>(u"readerStatus"_s);
        require(status, u"The Reader status is missing"_s);
        const QModelIndex unavailable = filter->mapFromSource(chapters->model()->index(19, 0));
        list->scrollTo(unavailable);
        QCoreApplication::processEvents();
        clickCanvas(list->viewport(), list->visualRect(unavailable).center());
        waitFor(u"unavailable chapter error"_s, [&] { return translate->isEnabled() && (status->text() == u"Generated chapter is unavailable."); });
        settle();
        require(!overlay->isVisible() && (chapters->currentData().toString() == u"volumes:21")
            && (list->currentIndex().data(Qt::UserRole).toString() == u"volumes:21")
            && (chapters->toolTip() == chapters->itemText(21)) && (text->toPlainText() == originalText)
            && (countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s) == readsAfterReopen + 1),
            u"A failed chapter read changed the original page or left stale chapter selection and tooltip"_s);
        clickCanvas(chapters, chapters->rect().center());
        waitFor(u"chapter picker after failed read"_s, [&] { return overlay->isVisible() && search->hasFocus(); });
        settle();
        require((list->currentIndex().data(Qt::UserRole).toString() == u"volumes:21")
            && (countEvents(logPath, u"volumes"_s, u"request"_s, u"reader.chapter"_s) == readsAfterReopen + 1),
            u"Reopening after a failed read restored the unavailable chapter"_s);
        search->setText(u"Volume 2"_s);
        list->setFocus();
        pressKey(QApplication::focusWidget(), Qt::Key_Down);
        QCoreApplication::processEvents();
        list->verticalScrollBar()->setValue(list->verticalScrollBar()->maximum() / 2);
        require((filter->rowCount() == 20) && (list->currentIndex().data(Qt::UserRole).toString() == u"volumes:22")
            && (list->verticalScrollBar()->value() > 0), u"The previous book did not establish a filtered and scrolled picker state"_s);
        pressKey(QApplication::focusWidget(), Qt::Key_Escape);
        waitFor(u"chapter picker closed after document reload"_s, [&] { return !overlay->isVisible(); });
        open(u"small-image"_s);
        waitForPage(0, 0, 1);
        const int smallReads = countEvents(logPath, u"small-image"_s, u"request"_s, u"reader.chapter"_s);
        clickCanvas(chapters, chapters->rect().center());
        overlay = reader.findChild<QWidget *>(u"readerChapterOverlay"_s);
        search = reader.findChild<QLineEdit *>(u"readerChapterSearch"_s);
        list = reader.findChild<QListView *>(u"readerChapterList"_s);
        require(overlay && search && list, u"The new book has no chapter picker"_s);
        waitFor(u"new book chapter picker"_s, [&] { return overlay->isVisible() && search->hasFocus(); });
        settle();
        const QModelIndex onlyChapter = list->model()->index(0, 0);
        require(search->text().isEmpty() && (list->model()->rowCount() == 1)
            && (onlyChapter.data(Qt::UserRole).toString() == u"small-image:chapter")
            && (list->currentIndex() == onlyChapter) && (list->verticalScrollBar()->value() == 0)
            && (chapters->currentData().toString() == u"small-image:chapter")
            && (countEvents(logPath, u"small-image"_s, u"request"_s, u"reader.chapter"_s) == smallReads),
            u"A new book inherited the previous book's filter, highlight or scroll position"_s);
        clickCanvas(list->viewport(), list->visualRect(onlyChapter).center());
        waitFor(u"new book exact chapter activation"_s, [&]
        {
            return !overlay->isVisible() && translate->isEnabled()
                && (countEvents(logPath, u"small-image"_s, u"request"_s, u"reader.chapter"_s) > smallReads);
        });
        settle();
        for (const QJsonValue &value : fixtureEvents(logPath))
        {
            const QJsonObject entry = value.toObject();
            if ((entry.value(u"documentId"_s).toString() == u"small-image") && (entry.value(u"method"_s).toString() == u"reader.chapter"))
                latestChapter = entry.value(u"params"_s).toObject().value(u"chapter"_s).toObject();
        }
        require((chapters->currentData().toString() == u"small-image:chapter")
            && (latestChapter.value(u"id"_s).toString() == u"small-image:chapter")
            && (countEvents(logPath, u"small-image"_s, u"request"_s, u"reader.chapter"_s) == smallReads + 1),
            u"The new book's chapter activation changed its ID or duplicated the read"_s);
        checks.append(u"Chapter search maps filtered rows to exact IDs; modal close and navigation retain query, selection and scroll"_s);
        UIThemeManager::instance()->previewColorScheme(ColorScheme::Light);
        reader.resize(700, 580);
    }

    for (const QString &scenario : {u"saved"_s, u"cancelled"_s, u"unchanged"_s})
    {
        open(scenario);
        waitForPage(0, 0, 3);
        next->click();
        waitForPage(0, 1, 3);
        settle();
        translate->click();
        waitFor(u"held multipage translation"_s, [&]
        {
            return (countEvents(logPath, scenario, u"held"_s) == 1) && showOriginal->isVisible();
        });
        waitForPage(0, 1, 3);
        requireFit(fixtureSizes.value(u"landscape"_s));

        bool handledDialog = false;
        bool selectedCorrection = false;
        QElapsedTimer dialogElapsed;
        dialogElapsed.start();
        QTimer dialogTimer;
        dialogTimer.setInterval(10);
        QObject::connect(&dialogTimer, &QTimer::timeout, &reader, [&]
        {
            for (QWidget *widget : QApplication::topLevelWidgets())
            {
                if (auto *dialog = qobject_cast<QInputDialog *>(widget); dialog && dialog->isVisible())
                {
                    handledDialog = true;
                    dialogTimer.stop();
                    if (scenario == u"cancelled")
                        dialog->reject();
                    else
                    {
                        if (scenario == u"saved")
                            dialog->setTextValue(u"Исправленный вручную перевод"_s);
                        dialog->accept();
                    }
                    return;
                }
                auto *menu = qobject_cast<QMenu *>(widget);
                if (!menu || !menu->isVisible())
                    continue;
                if (dialogElapsed.elapsed() > 3000)
                {
                    menu->close();
                    dialogTimer.stop();
                    return;
                }
                auto *action = menu->findChild<QAction *>(u"readerCorrectTranslation"_s);
                if (action && !selectedCorrection)
                {
                    selectedCorrection = true;
                    menu->setActiveAction(action);
                    pressKey(menu, Qt::Key_Return);
                    return;
                }
            }
        });
        dialogTimer.start();
        const QRect imageRect = images->property("imageRect").toRect();
        const QPoint point(imageRect.x() + (imageRect.width() / 2), imageRect.y() + (imageRect.height() * 7 / 20));
        QContextMenuEvent contextMenu(QContextMenuEvent::Mouse, point, canvas->mapToGlobal(point));
        QCoreApplication::sendEvent(canvas, &contextMenu);
        dialogTimer.stop();
        require(selectedCorrection && handledDialog, u"The region context menu did not open its native correction dialog"_s);
        waitFor(u"exactly one resumed ahead"_s, [&]
        {
            return countEvents(logPath, scenario, u"ahead-complete"_s) == 1;
        });
        settle();
        waitForPage(0, 1, 3);
        require(countEvents(logPath, scenario, u"request"_s, u"reader.ahead"_s) == 2,
            u"Correction did not resume exactly once, or ahead repeated after completion"_s);
        require(countEvents(logPath, scenario, u"cancelled-ahead"_s) == 1,
            u"Correction did not cancel its previously held ahead request"_s);
        require(countEvents(logPath, scenario, u"request"_s, u"reader.correct"_s) == (scenario == u"saved" ? 1 : 0),
            u"Correction persistence disagrees with dialog acceptance"_s);
        QStringList requests;
        bool savedCurrentPage = false;
        for (const QJsonValue &value : fixtureEvents(logPath))
        {
            const QJsonObject entry = value.toObject();
            if ((entry.value(u"documentId"_s).toString() != scenario)
                || (entry.value(u"event"_s).toString() != u"request"))
            {
                continue;
            }
            const QString method = entry.value(u"method"_s).toString();
            requests.append(method);
            const QJsonObject params = entry.value(u"params"_s).toObject();
            if (method == u"reader.ahead")
            {
                require(params.value(u"sourceId"_s).toString() == u"fixture-reader", u"Ahead lost its source context"_s);
                require(params.value(u"imageIndex"_s).toInt(-1) == 1, u"Ahead did not resume at the displayed manga page"_s);
            }
            if ((method == u"reader.position") && (params.value(u"imageIndex"_s).toInt(-1) == 1))
                savedCurrentPage = true;
            if (method == u"reader.correct")
            {
                require(params.value(u"text"_s).toString() == u"Исправленный вручную перевод", u"Edited text was not persisted"_s);
                require(params.value(u"regionId"_s).toString() == u"region-1", u"Correction targeted a different manga page"_s);
            }
        }
        require(savedCurrentPage, u"Manga page selection did not reach reader.position"_s);
        if (scenario == u"saved")
        {
            const int correction = requests.indexOf(u"reader.correct"_s);
            require((requests.indexOf(u"reader.ahead"_s) < correction)
                && (correction < requests.lastIndexOf(u"reader.ahead"_s)),
                u"Ahead resumed before correction completion instead of after it"_s);
            saveScreenshot(u"corrected-middle-page"_s);
        }

        const QImage translatedRender = canvas->grab().toImage();
        showOriginal->click();
        waitForPage(0, 1, 3);
        require(showOriginal->isChecked(), u"Original toggle did not retain the source page"_s);
        require(canvas->grab().toImage() != translatedRender, u"Original toggle did not restore original image pixels"_s);
        if (scenario == u"saved")
        {
            saveScreenshot(u"original-middle-page"_s);
            reader.resize(1040, 760);
            QCoreApplication::processEvents();
            waitForPage(0, 1, 3);
            requireFit(fixtureSizes.value(u"landscape"_s));
            require(showOriginal->isChecked(), u"Resizing replaced the selected original layer"_s);
            reader.resize(700, 580);
            QCoreApplication::processEvents();
            waitForPage(0, 1, 3);
        }
        showOriginal->click();
        waitForPage(0, 1, 3);
        require(!showOriginal->isChecked(), u"Translated layer did not return after original toggle"_s);
        require(canvas->grab().toImage() == translatedRender, u"Original toggle lost the corrected translated layer"_s);
        checks.append(QJsonObject {{u"name"_s, u"%1 correction resumes interrupted ahead once at the same page"_s.arg(scenario)},
            {u"aheadRequests"_s, 2}, {u"imageIndex"_s, 1}, {u"originalPreservesPage"_s, true}});
        evidence[u"checks"_s] = checks;
    }

    open(u"hangul"_s);
    waitFor(u"automatic Hangul translation"_s, [&]
    {
        return (countEvents(logPath, u"hangul"_s, u"ahead-complete"_s) == 1)
            && text->toPlainText().contains(u"Вместе"_s);
    });
    settle();
    require(countEvents(logPath, u"hangul"_s, u"request"_s, u"reader.ahead"_s) == 1,
        u"Automatic Hangul translation started a repeated request loop"_s);
    require(textSize->isVisible() && !pageNumber->isVisible(), u"Text chapters expose manga-only controls"_s);
    const int initialTextSize = textSize->value();
    textSize->setValue(initialTextSize + 2);
    require(qRound(text->document()->defaultFont().pointSizeF()) == initialTextSize + 2,
        u"Text size no longer changes prose typography"_s);
    require(QFontInfo(images->font()).family() == u"Neucha", u"Text size controls replaced the manga font"_s);
    textSize->setValue(initialTextSize);
    checks.append(u"Hangul automatically translates once; text typography remains independent of manga lettering"_s);
    evidence[u"checks"_s] = checks;

    open(u"text-position"_s);
    QScrollBar *textBar = text->verticalScrollBar();
    waitFor(u"scrollable generated prose"_s, [&] { return textBar->maximum() > 1000; });
    settle();
    textBar->setValue(qRound(textBar->maximum() * 0.42));
    const int textPosition = textBar->value();
    const auto recordTextGeometry = [&](const QString &stage)
    {
        const QJsonObject geometry {{u"event"_s, u"reader-geometry"_s}, {u"stage"_s, stage},
            {u"documentId"_s, u"text-position"_s}, {u"value"_s, textBar->value()},
            {u"maximum"_s, textBar->maximum()}, {u"expected"_s, textPosition},
            {u"viewportHeight"_s, text->viewport()->height()}};
        QFile file(logPath);
        require(file.open(QIODevice::WriteOnly | QIODevice::Append), u"Cannot record Reader position geometry"_s);
        file.write(QJsonDocument(geometry).toJson(QJsonDocument::Compact) + '\n');
    };
    recordTextGeometry(u"chosen"_s);
    chapters->setFocus();
    pressKey(chapters, Qt::Key_End);
    waitFor(u"second prose chapter"_s, [&] { return translate->isEnabled() && (chapters->currentIndex() == 1); });
    chapters->setFocus();
    pressKey(chapters, Qt::Key_Home);
    waitFor(u"prose chapter position"_s, [&]
    {
        return translate->isEnabled() && (chapters->currentIndex() == 0) && (qAbs(textBar->value() - textPosition) <= 2);
    });
    open(u"late-text-target"_s);
    open(u"text-position"_s);
    settle();
    recordTextGeometry(u"reopened"_s);
    require(qAbs(textBar->value() - textPosition) <= 2,
        u"Reopened prose position differs: expected %1, actual %2, maximum %3"_s
            .arg(textPosition).arg(textBar->value()).arg(textBar->maximum()));
    textBar->setValue(qRound(textBar->maximum() * 0.63));
    const double resizedTextOffset = double(textBar->value()) / textBar->maximum();
    textSize->setValue(initialTextSize + 1);
    open(u"late-text-target"_s);
    open(u"text-position"_s);
    settle();
    require(qAbs(textBar->value() - qRound(resizedTextOffset * textBar->maximum())) <= 2,
        u"Immediate navigation after a font change lost the latest prose position"_s);
    textSize->setValue(initialTextSize);
    open(u"late-text-target"_s);
    waitForPage(0, 0, 3);
    next->click();
    waitForPage(0, 1, 3);
    settle();
    require(countEvents(logPath, u"late-text-target"_s, u"request"_s, u"reader.position"_s) > 0,
        u"A stale text restoration blocked the next document's manga position"_s);
    checks.append(u"Prose chapter switches, reopen and font changes retain the latest position; stale text restore cannot block manga persistence"_s);

    open(u"positioned"_s);
    waitForPage(0, 1, 3);
    settle();
    waitForPage(0, 1, 3);
    require(countEvents(logPath, u"positioned"_s, u"request"_s, u"reader.ahead"_s) == 0,
        u"Restoring an untranslated original unexpectedly started translation"_s);
    require(!textSize->isVisible(), u"Text size controls are visible for manga pages"_s);
    checks.append(u"Saved imageIndex restores the selected manga page through image layout"_s);

    open(u"pager"_s);
    waitForPage(0, 0, 3);
    reader.activateWindow();
    waitFor(u"active Reader window after modal scenarios"_s, [&] { return reader.isActiveWindow(); });
    require(!previous->isEnabled() && next->isEnabled(), u"First-page navigation bounds are incorrect"_s);
    requireFit(fixtureSizes.value(u"portrait"_s));
    saveScreenshot(u"portrait-page"_s);
    clickCanvas(canvas, QPoint(canvas->width() / 4, canvas->height() / 2));
    waitForPage(0, 0, 3);
    clickCanvas(canvas, QPoint(canvas->width() * 3 / 4, canvas->height() / 2));
    waitForPage(0, 1, 3);
    require(images->hasFocus(), u"Canvas pointer navigation did not focus its keyboard controls: reader active=%1, active window=%2, application focus=%3, reader focus=%4"_s
        .arg(reader.isActiveWindow())
        .arg(QApplication::activeWindow() ? QApplication::activeWindow()->objectName() : u"none"_s)
        .arg(QApplication::focusWidget() ? QApplication::focusWidget()->objectName() : u"none"_s)
        .arg(reader.focusWidget() ? reader.focusWidget()->objectName() : u"none"_s));
    requireFit(fixtureSizes.value(u"landscape"_s));
    saveScreenshot(u"landscape-page"_s);
    pressKey(QApplication::focusWidget(), Qt::Key_Right);
    waitForPage(0, 2, 3);
    pressKey(QApplication::focusWidget(), Qt::Key_PageUp);
    waitForPage(0, 1, 3);
    pressKey(QApplication::focusWidget(), Qt::Key_PageDown);
    waitForPage(0, 2, 3);
    pressKey(QApplication::focusWidget(), Qt::Key_Left);
    waitForPage(0, 1, 3);
    chapters->setFocus();
    require(chapters->hasFocus(), u"Chapter selection could not take keyboard focus"_s);
    pressKey(QApplication::focusWidget(), Qt::Key_Left);
    waitForPage(0, 1, 3);
    next->click();
    waitForPage(0, 2, 3);
    reader.resize(640, 480);
    QCoreApplication::processEvents();
    const auto requireHeader = [&]
    {
        const auto bounds = [&](QWidget *control) { return QRect(control->mapTo(&reader, QPoint()), control->size()); };
        require(!back->text().isEmpty() && (back->parentWidget() == title->parentWidget()),
            u"Reader exit has no label or is separated from its title"_s);
        require((previous->parentWidget() == next->parentWidget()) && (previous->parentWidget() == pageNumber->parentWidget())
            && (bounds(previous).right() < bounds(pageNumber).left()) && (bounds(pageNumber).right() < bounds(next).left())
            && (bounds(next).right() >= reader.width() - 2),
            u"Reader page controls are not one ordered group at the right edge"_s);
        for (QWidget *control : QList<QWidget *> {back, title, chapters, previous, pageNumber, next, translate, showOriginal})
        {
            if (control->isVisible())
                require(reader.rect().contains(bounds(control)), u"A Reader header control is clipped at this window size"_s);
        }
        require(bounds(back).right() + 100 < bounds(previous).left(),
            u"Reader exit and previous-page arrows remain visually adjacent"_s);
    };
    requireHeader();
    requireFit(fixtureSizes.value(u"tall"_s));
    saveScreenshot(u"tall-page-compact"_s);
    reader.resize(800, 600);
    QCoreApplication::processEvents();
    requireHeader();
    saveScreenshot(u"reader-header-800"_s);
    reader.resize(1040, 760);
    QCoreApplication::processEvents();
    requireHeader();
    requireFit(fixtureSizes.value(u"tall"_s));
    waitForPage(0, 2, 3);
    saveScreenshot(u"tall-page-wide"_s);

    images->setFocus();
    require(images->hasFocus(), u"Manga canvas could not take keyboard focus"_s);
    pressKey(QApplication::focusWidget(), Qt::Key_Right);
    waitForPage(1, 0, 2);
    require(images->hasFocus(), u"Loading the next chapter lost canvas keyboard focus"_s);
    pressKey(QApplication::focusWidget(), Qt::Key_Right);
    waitForPage(1, 1, 2);
    pressKey(QApplication::focusWidget(), Qt::Key_Left);
    waitForPage(1, 0, 2);
    pressKey(QApplication::focusWidget(), Qt::Key_Left);
    waitForPage(0, 2, 3);
    require(images->hasFocus(), u"Loading the previous chapter lost canvas keyboard focus"_s);
    pressKey(QApplication::focusWidget(), Qt::Key_PageUp);
    waitForPage(0, 1, 3);
    pressKey(QApplication::focusWidget(), Qt::Key_PageUp);
    waitForPage(0, 0, 3);
    pressKey(QApplication::focusWidget(), Qt::Key_Right);
    waitForPage(0, 1, 3);
    pressKey(QApplication::focusWidget(), Qt::Key_Right);
    waitForPage(0, 2, 3);
    checks.append(u"Canvas focus survives both chapter boundaries; consecutive arrows and PageUp continue paging"_s);

    next->click();
    waitForPage(1, 0, 2);
    previous->click();
    waitForPage(0, 2, 3);
    chapters->setFocus();
    pressKey(QApplication::focusWidget(), Qt::Key_End);
    waitForPage(1, 0, 2);
    next->click();
    waitForPage(1, 1, 2);
    require(previous->isEnabled() && !next->isEnabled(), u"Last-page navigation bounds are incorrect"_s);
    clickCanvas(canvas, QPoint(canvas->width() * 3 / 4, canvas->height() / 2));
    waitForPage(1, 1, 2);
    chapters->setFocus();
    pressKey(QApplication::focusWidget(), Qt::Key_Home);
    waitForPage(0, 2, 3);
    require(chapters->hasFocus(), u"Loading a selected chapter lost the selector keyboard focus"_s);
    pressKey(QApplication::focusWidget(), Qt::Key_End);
    waitForPage(1, 1, 2);
    settle();
    open(u"positioned"_s);
    open(u"pager"_s);
    waitForPage(1, 1, 2);
    requireFit(fixtureSizes.value(u"portrait"_s));
    saveScreenshot(u"reopened-last-page"_s);
    checks.append(u"Canvas halves, focused arrows/PageUp/PageDown, toolbar and chapter boundaries select exact pages"_s);
    checks.append(u"Chapter combobox owns its keys and recalls each chapter page; resize and reopen retain imageIndex"_s);

    QJsonObject hashes;
    for (auto it = originalHashes.cbegin(); it != originalHashes.cend(); ++it)
    {
        require(fileHash(it.key()) == it.value(), u"Manual correction changed original image bytes"_s);
        hashes.insert(it.key(), QString::fromLatin1(it.value().toHex()));
    }
    evidence[u"checks"_s] = checks;
    evidence[u"fixtureEvents"_s] = fixtureEvents(logPath);
    evidence[u"originalSha256"_s] = hashes;
    evidence[u"scope"_s] = u"Actual Qt ReaderWidget and Content::Client with synthetic input and JSON-lines fixture; no live AI or physical pointer"_s;
    exerciseReaderSettings(spec, evidence);
}
