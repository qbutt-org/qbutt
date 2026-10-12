// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-settings-ui.h"

#include <functional>
#include <stdexcept>

#include <QApplication>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTimer>

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

void exerciseReaderSettingsUi(Content::ContentView *view, const QString &profileDir)
{
    auto *controller = view->findChild<Qbutt::ContentController *>();
    waitFor([&]
    {
        return !controller->property("libraryPending").toBool()
            && (controller->property("readerSettingsState").value<Qbutt::Reader::ReaderSettingsState>()
                == Qbutt::Reader::ReaderSettingsState::Ready);
    }, u"Reading settings before the UI fault"_s);

    auto *loader = view->rootObject()->findChild<QObject *>(u"contentReaderLoader"_s);
    auto *reader = loader ? loader->property("item").value<QQuickItem *>() : nullptr;
    require(reader, u"Current production reader is missing"_s);
    auto *width = reader->findChild<QQuickItem *>(u"readerWidthFit"_s);
    auto *height = reader->findChild<QQuickItem *>(u"readerHeightFit"_s);
    auto *error = reader->findChild<QQuickItem *>(u"readerSettingsError"_s);
    auto *retry = reader->findChild<QQuickItem *>(u"readerSettingsRetry"_s);
    require(width && height && error && retry, u"Production reading settings controls are missing"_s);
    require(!error->isVisible() && !retry->isVisible(), u"Healthy settings displayed an error"_s);

    const QString connection = u"reader-settings-ui"_s;
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
    const auto closeDatabase = qScopeGuard([&]
    {
        if (database.isOpen())
        {
            QSqlQuery cleanup(database);
            cleanup.exec(u"DROP TRIGGER IF EXISTS reader_settings_ui_abort"_s);
            database.close();
        }
        database = {};
        QSqlDatabase::removeDatabase(connection);
    });
    database.setDatabaseName(profileDir + u"/content-library.sqlite"_s);
    require(database.open(), database.lastError().text());
    const auto execute = [&](const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
    };
    const auto storedFit = [&]
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT fit FROM reader_settings WHERE id = 1"_s)
            && query.next(), query.lastError().text());
        return query.value(0).toInt();
    };
    require(storedFit() == 0, u"UI fault did not start with saved Height"_s);
    execute(u"CREATE TRIGGER reader_settings_ui_abort BEFORE UPDATE ON reader_settings "
        "BEGIN SELECT RAISE(ABORT, 'reading settings UI fault'); END"_s);
    width->forceActiveFocus(Qt::TabFocusReason);
    require(QMetaObject::invokeMethod(width, "clicked"), u"QML Width action failed"_s);
    waitFor([&]
    {
        return controller->property("readerSettingsState").value<Qbutt::Reader::ReaderSettingsState>()
            == Qbutt::Reader::ReaderSettingsState::Failed;
    }, u"Reading settings UI save fault"_s);
    require(width->property("checked").toBool() && !height->property("checked").toBool()
        && (controller->property("readerFit").value<Qbutt::Reader::ReaderFit>() == Qbutt::Reader::ReaderFit::Width)
        && (storedFit() == 0), u"Failed settings write changed the applied fit or committed value"_s);
    const QString message = QCoreApplication::translate("ContentController", "Could not save reading settings.");
    require(error->isVisible() && retry->isVisible() && retry->isEnabled()
        && (error->property("text").toString() == message), u"Settings failure has no specific usable retry"_s);

    const bool favorite = controller->property("selectedFavorite").toBool();
    controller->setSelectedFavorite(!favorite);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Unrelated favorite write"_s);
    require((controller->property("selectedFavorite").toBool() == !favorite)
        && controller->property("libraryErrorText").toString().isEmpty()
        && error->isVisible() && retry->isVisible() && (storedFit() == 0),
        u"Unrelated library success cleared the settings failure"_s);

    execute(u"DROP TRIGGER reader_settings_ui_abort"_s);
    retry->forceActiveFocus(Qt::TabFocusReason);
    require(QMetaObject::invokeMethod(retry, "clicked"), u"QML settings Retry action failed"_s);
    waitFor([&]
    {
        return controller->property("readerSettingsState").value<Qbutt::Reader::ReaderSettingsState>()
            == Qbutt::Reader::ReaderSettingsState::Ready;
    }, u"QML exact settings retry"_s);
    require((storedFit() == 1) && width->property("checked").toBool()
        && !error->isVisible() && !retry->isVisible(), u"Exact UI retry did not commit the applied Width"_s);
    require(controller->setReaderFit(Qbutt::Reader::ReaderFit::Height), u"Restore Height was rejected"_s);
    controller->setSelectedFavorite(favorite);
    waitFor([&]
    {
        return !controller->property("libraryPending").toBool()
            && (controller->property("readerSettingsState").value<Qbutt::Reader::ReaderSettingsState>()
                == Qbutt::Reader::ReaderSettingsState::Ready);
    }, u"Reading UI fault cleanup"_s);
    require((storedFit() == 0) && (controller->property("selectedFavorite").toBool() == favorite),
        u"Reading UI fault did not restore its fixture state"_s);
}
