// SPDX-License-Identifier: GPL-2.0-or-later
#include "local-books-ui.h"

#include <functional>
#include <stdexcept>

#include <QByteArray>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QMetaObject>
#include <QObject>
#include <QQuickItem>
#include <QTimer>
#include <QUrl>

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
        timer.setInterval(20);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if (condition() || (elapsed.elapsed() >= 10'000))
                loop.quit();
        });
        if (!condition())
        {
            timer.start();
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }
}

void exerciseLocalBookImportUi(Content::ContentView *view, const QString &profileDir)
{
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Content controller is missing"_s);
    waitFor([&]
    {
        return controller->property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready;
    }, u"Local book library"_s);

    const auto createBook = [&](const QString &name, const QByteArray &bytes)
    {
        const QString path = profileDir + u"/"_s + name;
        QFile file(path);
        require(file.open(QIODevice::WriteOnly) && file.write(bytes) == bytes.size(),
            u"Could not write generated local book"_s);
        file.close();
        return QUrl::fromLocalFile(path);
    };
    const QUrl valid = createBook(u"valid.txt"_s, "First chapter.\nSecond paragraph.");
    const QUrl oversized = createBook(u"oversized.txt"_s, QByteArray(4 * 1024 * 1024 + 1, 'x'));

    controller->importLocalBook(valid);
    waitFor([&]
    {
        return controller->property("readerOpen").toBool()
            && !controller->property("readerBusy").toBool()
            && !controller->property("readerChapterId").toString().isEmpty();
    }, u"Valid local book opening"_s);
    const QString chapter = controller->property("readerChapterId").toString();
    const QString title = controller->property("readerTitle").toString();
    auto *popup = view->rootObject()->findChild<QObject *>(u"contentLocalBookErrorPopup"_s);
    auto *message = popup ? popup->findChild<QQuickItem *>(u"contentLocalBookError"_s) : nullptr;
    auto *close = popup ? popup->findChild<QQuickItem *>(u"contentLocalBookErrorClose"_s) : nullptr;
    require(popup && message && close, u"Local book error popup is missing"_s);

    controller->importLocalBook(oversized);
    waitFor([&]
    {
        return controller->property("localBookError").toString() == u"large"_s
            && popup->property("visible").toBool();
    }, u"Invalid local book error"_s);
    require(message->isVisible() && !message->property("text").toString().isEmpty()
        && controller->property("readerOpen").toBool()
        && controller->property("readerChapterId").toString() == chapter
        && controller->property("readerTitle").toString() == title,
        u"Import error hid or replaced the open chapter"_s);

    require(QMetaObject::invokeMethod(close, "clicked"), u"Error close action failed"_s);
    waitFor([&] { return !popup->property("visible").toBool(); }, u"Error dismissal"_s);
    require(controller->property("readerOpen").toBool()
        && controller->property("readerChapterId").toString() == chapter
        && controller->property("readerTitle").toString() == title,
        u"Dismissing the import error changed the open chapter"_s);
    qInfo().noquote() << u"PASS local_book_import_ui visible_error=ok same_chapter=ok dismiss=ok"_s;
}
