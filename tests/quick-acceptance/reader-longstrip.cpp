// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-longstrip.h"

#include <atomic>
#include <chrono>
#include <functional>
#include <stdexcept>
#include <thread>

#include <QCryptographicHash>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QFileInfo>
#include <QFont>
#include <QImage>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLinearGradient>
#include <QMetaObject>
#include <QPainter>
#include <QQuickItem>
#include <QTimer>
#include <QUrl>

#ifdef Q_OS_WIN
#include <qt_windows.h>
#include <psapi.h>
#endif

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &condition, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        QEventLoop loop;
        QTimer timer;
        timer.setInterval(10);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if (condition() || elapsed.elapsed() >= 15'000)
                loop.quit();
        });
        if (!condition())
        {
            timer.start();
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }

    quint64 privateBytes()
    {
#ifdef Q_OS_WIN
        PROCESS_MEMORY_COUNTERS_EX counters {};
        counters.cb = sizeof(counters);
        if (GetProcessMemoryInfo(GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS *>(&counters), sizeof(counters)))
            return counters.PrivateUsage;
#endif
        return 0;
    }

    class MemorySample
    {
    public:
        MemorySample()
            : baseline {privateBytes()}
            , peak {baseline}
            , worker {[this](const std::stop_token stop)
            {
                while (!stop.stop_requested())
                {
                    const quint64 sample = privateBytes();
                    quint64 observed = peak.load();
                    while ((sample > observed) && !peak.compare_exchange_weak(observed, sample)) {}
                    std::this_thread::sleep_for(std::chrono::milliseconds(2));
                }
            }}
        {
            require(baseline > 0, u"Native private memory counter is unavailable"_s);
        }

        void stop()
        {
            worker.request_stop();
            worker.join();
        }

        const quint64 baseline;
        std::atomic<quint64> peak;

    private:
        std::jthread worker;
    };

    void createLongstrip(const QString &path, const QString &shortPath)
    {
        QImage image(800, 20'000, QImage::Format_RGB32);
        QPainter painter(&image);
        QLinearGradient gradient(0, 0, 800, 20'000);
        gradient.setColorAt(0, QColor(205, 227, 246));
        gradient.setColorAt(0.5, QColor(231, 189, 140));
        gradient.setColorAt(1, QColor(62, 94, 126));
        painter.fillRect(image.rect(), gradient);
        painter.setFont(QFont(u"Segoe UI"_s, 24));
        for (int top = 0; top < image.height(); top += 500)
        {
            painter.setPen(QPen(QColor(37, 49, 64), 2));
            painter.drawLine(0, top, 799, top);
            painter.drawText(30, top + 55, u"Original y=%1"_s.arg(top));
        }
        painter.setPen(QPen(QColor(37, 49, 64), 4));
        painter.setBrush(QColor(255, 248, 225));
        painter.drawRoundedRect(QRect(90, 1910, 620, 310), 35, 35);
        painter.drawText(QRect(130, 1950, 540, 230), Qt::AlignCenter | Qt::TextWordWrap,
            u"This complete speech balloon crosses the tile boundary."_s);
        painter.end();
        require(image.save(path, "PNG"), u"Could not create the generated long PNG"_s);
        require(image.copy(0, 0, 800, 1280).save(shortPath, "PNG"), u"Could not create the ordinary PNG"_s);
    }

    QByteArray digest(const QString &path)
    {
        QFile file(path);
        require(file.open(QIODevice::ReadOnly), u"Could not read the immutable PNG"_s);
        QCryptographicHash hash(QCryptographicHash::Sha256);
        require(hash.addData(&file), u"Could not hash the immutable PNG"_s);
        return hash.result().toHex();
    }

    QQuickItem *findItem(Content::ContentView *view, const QString &name)
    {
        QList<QQuickItem *> candidates {view->rootObject()};
        for (qsizetype index = 0; index < candidates.size(); ++index)
        {
            QQuickItem *candidate = candidates[index];
            if (candidate->objectName() == name)
                return candidate;
            candidates.append(candidate->childItems());
        }
        return nullptr;
    }

    QQuickItem *item(Content::ContentView *view, const QString &name)
    {
        QQuickItem *result = findItem(view, name);
        require(result, u"Reader item is missing: "_s + name);
        return result;
    }

    int checkTiles(QQuickItem *image)
    {
        require(image->property("tiled").toBool(), u"Long Width image did not use viewport tiles"_s);
        QList<QQuickItem *> candidates = image->childItems();
        for (qsizetype index = 0; index < candidates.size(); ++index)
            candidates.append(candidates[index]->childItems());
        const qreal scale = image->property("displayScale").toReal();
        int count = 0;
        for (QQuickItem *tile : candidates)
        {
            if (!tile->objectName().startsWith(u"readerTileImage")
                || !tile->parentItem()->property("active").toBool())
                continue;
            ++count;
            const QRectF crop(0, tile->parentItem()->property("sourceTop").toInt(), 800,
                tile->parentItem()->property("sourceHeight").toInt());
            require((crop.x() == 0) && (crop.width() == 800) && (crop.y() >= 0)
                && (crop.height() > 0) && (crop.height() <= 2048)
                && (crop.width() * crop.height() <= 2'000'000)
                && (crop.bottom() <= 20'000), u"Tile source geometry exceeds its native bounds"_s);
            require(tile->property("sourceSize").toSize() == crop.size().toSize(),
                u"Native tile dimensions differ from the original source crop"_s);
            require(qAbs(tile->mapToItem(image, QPointF()).y()
                - image->property("imageTop").toReal() - crop.y() * scale) < 0.1,
                u"Tile order or global source position changed"_s);
        }
        require(count > 0 && count <= 3, u"Visible tile image count exceeds the viewport bounds"_s);
        return count;
    }

    QJsonObject measureTransition(Content::ContentView *view, Qbutt::ContentController *controller, const QString &archive)
    {
        controller->closeReader();
        controller->setReaderMode(Qbutt::Reader::ReaderMode::Continuous);
        controller->setReaderFit(Qbutt::Reader::ReaderFit::Width);
        controller->importLocalBook(QUrl::fromLocalFile(archive));
        waitFor([&]
        {
            return controller->property("readerOpen").toBool()
                && !controller->property("readerBusy").toBool()
                && controller->property("readerPageCount").toInt() == 2;
        }, u"Adjacent long originals"_s);
        QQuickItem *viewport = item(view, u"readerContinuousViewport"_s);
        QQuickItem *continuous = item(view, u"readerContinuousScroll"_s);
        waitFor([&]
        {
            QQuickItem *first = findItem(view, u"readerStripImage0"_s);
            return first && (first->property("status").toInt() == 1);
        }, u"First adjacent original decode"_s);
        QQuickItem *first = item(view, u"readerStripImage0"_s);
        const qreal seam = first->height() - viewport->height() / 2;
        continuous->setProperty("userScrolling", true);
        QElapsedTimer scroll;
        scroll.start();
        viewport->setProperty("contentY", seam);
        const auto ready = [&]
        {
            QQuickItem *firstImage = findItem(view, u"readerStripImage0"_s);
            QQuickItem *secondImage = findItem(view, u"readerStripImage1"_s);
            return firstImage && secondImage && (firstImage->property("status").toInt() == 1)
                && (secondImage->property("status").toInt() == 1);
        };
        waitFor(ready, u"Both original pages at their shared seam"_s);
        const qint64 coldMs = scroll.elapsed();
        qint64 warmMs = 0;
        for (const int delta : {-200, 200, -180, 180, -120, 120, -60, 60, 0})
        {
            continuous->setProperty("userScrolling", true);
            scroll.restart();
            viewport->setProperty("contentY", seam + delta);
            waitFor(ready, u"Warm adjacent original seam"_s);
            warmMs = qMax(warmMs, scroll.elapsed());
        }
        require(checkTiles(item(view, u"readerStripImage0"_s)) <= 2
            && checkTiles(item(view, u"readerStripImage1"_s)) <= 2,
            u"Adjacent original pages retained excessive visible tiles"_s);
        return {{u"coldMs"_s, coldMs}, {u"warmMaximumMs"_s, warmMs}, {u"moves"_s, 9}};
    }

    void capture(Content::ContentView *view, const QString &path)
    {
        QEventLoop frame;
        QTimer::singleShot(100, &frame, &QEventLoop::quit);
        frame.exec();
        require(view->grabFramebuffer().save(path), u"Native longstrip screenshot failed"_s);
    }
}

void exerciseReaderLongstrip(Content::ContentView *view, const QString &profileDir)
{
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"ContentController is missing"_s);
    waitFor([&]
    {
        return controller->property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Ready;
    }, u"Longstrip library"_s);
    controller->setReaderMode(Qbutt::Reader::ReaderMode::Paged);
    controller->setReaderFit(Qbutt::Reader::ReaderFit::Height);
    view->window()->resize(1440, 900);
    const QString input = profileDir + u"/longstrip-800x20000.png"_s;
    const QString shortInput = profileDir + u"/ordinary-800x1280.png"_s;
    createLongstrip(input, shortInput);
    const QByteArray originalDigest = digest(input);
    const auto settled = [&]
    {
        return !controller->property("readerProgressPending").toBool()
            && !controller->property("libraryPending").toBool();
    };
    MemorySample memory;
    controller->importLocalBook(QUrl::fromLocalFile(input));
    waitFor([&]
    {
        return controller->property("readerOpen").toBool() && !controller->property("readerBusy").toBool()
            && controller->property("readerImageHeight").toInt() == 20'000;
    }, u"Long original acquisition"_s);
    require(controller->property("readerImageWidth").toInt() == 800
        && controller->property("readerPageCount").toInt() == 1, u"Source page was split or resized"_s);
    const QUrl originalUrl = controller->property("readerPageUrl").toUrl();
    require(originalUrl.isLocalFile() && digest(originalUrl.toLocalFile()) == originalDigest,
        u"Reader changed the original encoded bytes"_s);

    controller->setReaderFit(Qbutt::Reader::ReaderFit::Width);
    auto *image = item(view, u"readerPageImage"_s);
    auto *viewport = item(view, u"readerPageViewport"_s);
    auto *canvas = item(view, u"readerCanvas"_s);
    int maximumTiles = 0;
    qint64 maximumScrollMs = 0;
    qint64 initialDecodeMs = 0;
    for (const int sourceTop : {0, 1800, 7300, 15'800, 19'800})
    {
        QElapsedTimer scroll;
        scroll.start();
        const qreal target = qMin(sourceTop * image->property("displayScale").toReal(),
            viewport->property("contentHeight").toReal() - viewport->height());
        viewport->setProperty("contentY", target);
        waitFor([&] { return image->property("status").toInt() == 1; }, u"Visible longstrip decode"_s);
        if (sourceTop == 0)
            initialDecodeMs = scroll.elapsed();
        else
            maximumScrollMs = qMax(maximumScrollMs, scroll.elapsed());
        maximumTiles = qMax(maximumTiles, checkTiles(image));
        require(QMetaObject::invokeMethod(canvas, "saveScroll"), u"Longstrip position save did not run"_s);
        waitFor(settled, u"Longstrip progress write"_s);
        require(qAbs(controller->property("readerScrollOffset").toReal() - target / image->height()) < 0.000001,
            u"Position stopped referring to the logical original page"_s);
        if (sourceTop == 1800)
        {
            require(maximumTiles >= 2, u"Speech seam did not exercise two visible tiles"_s);
            capture(view, profileDir + u"/longstrip-seam.png"_s);
        }
    }
    capture(view, profileDir + u"/longstrip-bottom.png"_s);

    viewport->setProperty("contentY", image->height() * 0.41);
    waitFor([&] { return image->property("status").toInt() == 1; }, u"Confirmed restore anchor"_s);
    QMetaObject::invokeMethod(canvas, "saveScroll");
    waitFor(settled, u"Confirmed restore anchor save"_s);
    const double offset = controller->property("readerScrollOffset").toDouble();
    viewport->setProperty("contentY", image->height() * 0.8);
    controller->closeReader();
    waitFor([&] { return !QFileInfo::exists(originalUrl.toLocalFile()); }, u"Cancelled longstrip cache cleanup"_s);
    controller->importLocalBook(QUrl::fromLocalFile(input));
    waitFor([&]
    {
        return controller->property("readerOpen").toBool()
            && !controller->property("readerBusy").toBool()
            && controller->property("readerImageHeight").toInt() == 20'000
            && !controller->property("readerPageUrl").toUrl().isEmpty();
    }, u"Longstrip reopen"_s);
    image = item(view, u"readerPageImage"_s);
    viewport = item(view, u"readerPageViewport"_s);
    waitFor([&]
    {
        return image->property("status").toInt() == 1
            && qAbs(viewport->property("contentY").toReal() - image->height() * offset) < 1;
    }, u"Exact source-relative restore"_s);
    require(controller->property("readerPageCount").toInt() == 1
        && controller->property("readerScrollOffset").toDouble() == offset,
        u"Tile cancellation replaced the confirmed logical position"_s);

    controller->setReaderMode(Qbutt::Reader::ReaderMode::Continuous);
    waitFor([&] { return findItem(view, u"readerStripImage0"_s); },
        u"Continuous image creation"_s);
    image = item(view, u"readerStripImage0"_s);
    viewport = item(view, u"readerContinuousViewport"_s);
    waitFor([&] { return image->property("status").toInt() == 1; }, u"Continuous longstrip tiles"_s);
    maximumTiles = qMax(maximumTiles, checkTiles(image));
    capture(view, profileDir + u"/longstrip-continuous.png"_s);
    require(controller->property("readerPageCount").toInt() == 1
        && controller->property("readerScrollOffset").toDouble() == offset,
        u"Mode change rewrote the original position"_s);
    auto *continuous = item(view, u"readerContinuousScroll"_s);
    int deferredScrolls = 0;
    for (const double nextOffset : {0.12, 0.73})
    {
        const double confirmedOffset = controller->property("readerScrollOffset").toDouble();
        continuous->setProperty("userScrolling", true);
        QElapsedTimer scroll;
        scroll.start();
        viewport->setProperty("contentY", image->height() * nextOffset);
        require(QMetaObject::invokeMethod(continuous, "saveScroll"), u"Continuous viewport request did not run"_s);
        if (image->property("status").toInt() != 1)
        {
            ++deferredScrolls;
            require(controller->property("readerScrollOffset").toDouble() == confirmedOffset,
                u"Continuous progress accepted an undecoded visible tile"_s);
        }
        waitFor([&]
        {
            return image->property("status").toInt() == 1
                && qAbs(controller->property("readerScrollOffset").toDouble() - nextOffset) < 0.000001
                && settled();
        }, u"Continuous progress after visible decode"_s);
        maximumScrollMs = qMax(maximumScrollMs, scroll.elapsed());
        maximumTiles = qMax(maximumTiles, checkTiles(image));
    }
    require(deferredScrolls > 0, u"Continuous scroll did not exercise an undecoded visible tile"_s);

    controller->setReaderFit(Qbutt::Reader::ReaderFit::Height);
    waitFor([&]
    {
        QQuickItem *overview = findItem(view, u"readerStripImage0"_s);
        return overview && (overview->property("status").toInt() == 1);
    }, u"Longstrip Height overview"_s);
    image = item(view, u"readerStripImage0"_s);
    require(!image->property("tiled").toBool() && image->height() <= viewport->height() + 1,
        u"Height overview retained all longstrip tiles"_s);
    memory.stop();
    const QString mixedArchive = qEnvironmentVariable("QBUTT_LONGSTRIP_MIXED_ARCHIVE");
    if (!mixedArchive.isEmpty())
    {
        controller->closeReader();
        controller->setReaderMode(Qbutt::Reader::ReaderMode::Paged);
        controller->importLocalBook(QUrl::fromLocalFile(mixedArchive));
        waitFor([&]
        {
            return controller->property("readerOpen").toBool()
                && !controller->property("readerBusy").toBool()
                && controller->property("readerPageCount").toInt() == 2
                && controller->property("readerImageHeight").toInt() == 20'000;
        }, u"Mixed long and ordinary originals"_s);
        controller->selectReaderPage(1);
        waitFor([&]
        {
            return !controller->property("readerBusy").toBool()
                && controller->property("readerImageHeight").toInt() == 1280;
        }, u"Ordinary page in the same chapter"_s);
        controller->selectReaderPage(0);
        waitFor([&] { return !controller->property("readerBusy").toBool(); }, u"Return to long original"_s);
        controller->setReaderFit(Qbutt::Reader::ReaderFit::Width);
        controller->setReaderMode(Qbutt::Reader::ReaderMode::Continuous);
        waitFor([&]
        {
            QQuickItem *current = findItem(view, u"readerStripImage0"_s);
            return current && (current->property("status").toInt() == 1);
        }, u"Mixed Continuous image"_s);
        item(view, u"readerContinuousScroll"_s)->setProperty("userScrolling", true);
        item(view, u"readerContinuousViewport"_s)->setProperty("contentY",
            item(view, u"readerStripImage0"_s)->height());
        waitFor([&] { return controller->property("readerPageIndex").toInt() == 1; },
            u"Continuous confirmed ordinary-page anchor"_s);
    }
    const QString transitionArchive = qEnvironmentVariable("QBUTT_LONGSTRIP_TRANSITION_ARCHIVE");
    const QJsonObject transition = transitionArchive.isEmpty() ? QJsonObject()
        : measureTransition(view, controller, transitionArchive);

    controller->closeReader();
    controller->importLocalBook(QUrl::fromLocalFile(shortInput));
    controller->setReaderMode(Qbutt::Reader::ReaderMode::Paged);
    waitFor([&]
    {
        return !controller->property("readerBusy").toBool()
            && controller->property("readerImageHeight").toInt() == 1280;
    }, u"Ordinary original acquisition"_s);
    image = item(view, u"readerPageImage"_s);
    viewport = item(view, u"readerPageViewport"_s);
    for (const QSize size : {QSize(1280, 720), QSize(1440, 900)})
    {
        view->window()->resize(size);
        for (const Qbutt::Reader::ReaderFit fit : {Qbutt::Reader::ReaderFit::Width, Qbutt::Reader::ReaderFit::Height})
        {
            controller->setReaderFit(fit);
            waitFor([&] { return image->property("status").toInt() == 1; }, u"Ordinary original fit"_s);
            require(!image->property("tiled").toBool()
                && image->property("paintedWidth").toReal() <= viewport->width() + 1
                && (fit != Qbutt::Reader::ReaderFit::Height
                    || image->property("paintedHeight").toReal() <= viewport->height() + 1),
                u"Ordinary original fit regressed"_s);
        }
    }
    const QUrl shortUrl = controller->property("readerPageUrl").toUrl();
    controller->closeReader();
    waitFor([&] { return !QFileInfo::exists(shortUrl.toLocalFile()); }, u"Ordinary cache cleanup"_s);

    const quint64 peak = memory.peak.load();
    QJsonObject receipt {
        {u"width"_s, 800}, {u"height"_s, 20'000}, {u"logicalPages"_s, 1},
        {u"originalDigest"_s, QString::fromLatin1(originalDigest)},
        {u"maximumVisibleTiles"_s, maximumTiles}, {u"maximumScrollMs"_s, maximumScrollMs},
        {u"deferredContinuousScrolls"_s, deferredScrolls},
        {u"initialDecodeMs"_s, initialDecodeMs},
        {u"privateBaselineBytes"_s, qint64(memory.baseline)},
        {u"privatePeakBytes"_s, qint64(peak)}, {u"privateGrowthBytes"_s, qint64(peak - memory.baseline)},
        {u"sourceRgbaBytes"_s, 800 * 20'000 * 4},
        {u"position"_s, offset},
        {u"adjacentPages"_s, transition},
        {u"mixedChapter"_s, !mixedArchive.isEmpty()},
    };
    QFile receiptFile(profileDir + u"/reader-longstrip-receipt.json"_s);
    require(receiptFile.open(QIODevice::WriteOnly)
        && receiptFile.write(QJsonDocument(receipt).toJson()) > 0, u"Longstrip receipt write failed"_s);
    qInfo().noquote() << u"PASS reader_longstrip original=800x20000 logical_pages=1 tiles_max=%1 scroll_max_ms=%2 private_growth_bytes=%3 position=exact cancel=ok"_s
        .arg(maximumTiles).arg(maximumScrollMs).arg(peak - memory.baseline);
}
