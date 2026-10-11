// SPDX-License-Identifier: GPL-2.0-or-later
#include <atomic>
#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QCommandLineParser>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QLocale>
#include <QMainWindow>
#include <QPointer>
#include <QQuickItem>
#include <QSet>
#include <QStandardPaths>
#include <QStatusBar>
#include <QStyleHints>
#include <QThread>
#include <QTimer>
#include <QUrl>
#include <QUrlQuery>
#include <QVariant>

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(const bool accepted, const QString &message)
    {
        if (!accepted)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &condition, const int timeout, const QString &operation)
    {
        if (condition())
            return;

        QElapsedTimer elapsed;
        elapsed.start();
        QEventLoop loop;
        QTimer timer;
        timer.setInterval(20);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if ((elapsed.elapsed() >= timeout) || condition())
                loop.quit();
        });
        timer.start();
        loop.exec();
        require((elapsed.elapsed() < timeout) && condition(), operation + u" timed out"_s);
    }

    int role(QAbstractItemModel *model, const QByteArray &name)
    {
        const QHash<int, QByteArray> roles = model->roleNames();
        for (auto iterator = roles.cbegin(); iterator != roles.cend(); ++iterator)
        {
            if (iterator.value() == name)
                return iterator.key();
        }
        throw std::runtime_error("Missing model role: " + name.toStdString());
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        return model->data(model->index(row, 0), role(model, name)).toString();
    }

    QAbstractItemModel *modelProperty(QObject *controller, const char *name)
    {
        auto *model = qobject_cast<QAbstractItemModel *>(controller->property(name).value<QObject *>());
        require(model, u"Controller property is not a QAbstractItemModel: %1"_s.arg(QString::fromLatin1(name)));
        require(model->thread() == qApp->thread(), u"Model has the wrong thread affinity"_s);
        return model;
    }

    void exercise(QPointer<QMainWindow> window, QPointer<Content::ContentView> view)
    {
        require(window && view, u"Window closed before the scenario started"_s);
        waitFor([view]
        {
            return !view || (view->status() == QQuickWidget::Ready) || (view->status() == QQuickWidget::Error);
        }, 10'000, u"QML load"_s);
        require(view, u"Window closed before the scenario completed"_s);
        require(view->status() == QQuickWidget::Ready, u"Production ContentView did not load"_s);
        require(view->rootObject(), u"ContentView has no root object"_s);

        QPointer<Qbutt::ContentController> controller = view->findChild<Qbutt::ContentController *>();
        require(controller, u"Production ContentController is missing"_s);
        require(controller->thread() == qApp->thread(), u"Controller has the wrong thread affinity"_s);
        QPointer<QAbstractItemModel> works = modelProperty(controller, "works");
        QPointer<QAbstractItemModel> variants = modelProperty(controller, "variants");
        require(works != variants, u"Work and variant collections share one model"_s);

        QObject observation;
        std::atomic_bool wrongThread = false;
        int modelChanges = 0;
        const auto changed = [&]
        {
            if (QThread::currentThread() != qApp->thread())
                wrongThread.store(true);
            else
                ++modelChanges;
        };
        for (QAbstractItemModel *model : {works.data(), variants.data()})
        {
            QObject::connect(model, &QAbstractItemModel::rowsInserted, &observation, changed, Qt::DirectConnection);
            QObject::connect(model, &QAbstractItemModel::rowsRemoved, &observation, changed, Qt::DirectConnection);
            QObject::connect(model, &QAbstractItemModel::modelReset, &observation, changed, Qt::DirectConnection);
            QObject::connect(model, &QAbstractItemModel::dataChanged, &observation, changed, Qt::DirectConnection);
        }

        controller->search(u"Naruto"_s);
        require(controller->property("busy").toBool(), u"Search did not enter its busy state"_s);
        controller->cancelSearch();
        require(!controller->property("busy").toBool(), u"Cancel did not leave the busy state"_s);

        controller->search(u"Frieren"_s);
        require(controller->property("busy").toBool(), u"New search did not start after cancellation"_s);
        waitFor([&] { return !controller || !controller->property("busy").toBool(); }, 50'000, u"Live AniLiberty search"_s);
        require(controller, u"Window closed during search"_s);
        require(controller->property("errorText").toString().isEmpty(),
            u"Search failed: %1"_s.arg(controller->property("errorText").toString()));
        require(!wrongThread.load(), u"Model mutation was emitted outside the GUI thread"_s);
        require((works->rowCount() > 0) && (works->rowCount() <= 500), u"Search returned an invalid row count"_s);

        QSet<QString> identities;
        QString selectedWork;
        role(works, "year");
        for (int row = 0; row < works->rowCount(); ++row)
        {
            const QString id = value(works, row, "workId");
            const QString title = value(works, row, "title");
            require(!id.isEmpty() && !identities.contains(id), u"Work identity is missing or duplicated"_s);
            require(!title.isEmpty(), u"Search returned an unnamed work"_s);
            require(title.contains(u"Frieren"_s, Qt::CaseInsensitive) || title.contains(u"Фрирен"_s, Qt::CaseInsensitive),
                u"Cancelled or unrelated search results entered the new search"_s);
            require(value(works, row, "source") == u"AniLiberty", u"Incorrect work source role"_s);
            require(value(works, row, "category") == u"anime", u"Incorrect work category role"_s);
            identities.insert(id);
            if (id == u"aniliberty:9542")
                selectedWork = id;
        }
        require(!selectedWork.isEmpty(), u"Live catalog did not return the expected publication 9542"_s);

        QPointer<QObject> resultView = view->rootObject()->findChild<QObject *>(u"contentResults"_s);
        require(resultView, u"The production results view is missing"_s);
        waitFor([&] { return !resultView || (resultView->property("count").toInt() == works->rowCount()); }, 2'000, u"Work model binding"_s);
        require(resultView, u"Window closed before work model binding"_s);

        controller->selectWork(selectedWork);
        waitFor([&] { return !controller || !controller->property("inspecting").toBool(); }, 50'000, u"Publication inspection"_s);
        require(controller, u"Window closed during inspection"_s);
        require(controller->property("errorText").toString().isEmpty(),
            u"Inspection failed: %1"_s.arg(controller->property("errorText").toString()));
        require(controller->property("selectedWorkId").toString() == selectedWork, u"Inspection changed publication identity"_s);
        require(!controller->property("selectedWorkTitle").toString().isEmpty(), u"Inspected title is missing"_s);
        require(variants->rowCount() > 0, u"Inspection did not expose torrent variants"_s);

        identities.clear();
        for (const QByteArray &name : {QByteArray("quality"), QByteArray("sizeText"), QByteArray("seedersText"), QByteArray("infoHash")})
            role(variants, name);
        for (int row = 0; row < variants->rowCount(); ++row)
        {
            const QString id = value(variants, row, "variantId");
            require(id.startsWith(selectedWork + u":torrent:"_s) && !identities.contains(id),
                u"Variant identity is duplicated or belongs to another publication"_s);
            require(!value(variants, row, "label").isEmpty(), u"Torrent variant has no label"_s);
            require(value(variants, row, "source") == u"AniLiberty", u"Incorrect variant source role"_s);
            require(value(variants, row, "kind") == u"torrent", u"An unsupported action entered the torrent slice"_s);
            identities.insert(id);
        }

        QPointer<QObject> variantView = view->rootObject()->findChild<QObject *>(u"contentVariants"_s);
        require(variantView, u"The production variants view is missing"_s);
        waitFor([&] { return !variantView || (variantView->property("count").toInt() == variants->rowCount()); }, 2'000, u"Variant model binding"_s);
        require(variantView, u"Window closed before variant model binding"_s);

        const QString selectedVariant = value(variants, 0, "variantId");
        const QString expectedHash = value(variants, 0, "infoHash").toLower();
        require(expectedHash.size() == 40, u"Selected torrent has no verifiable infohash"_s);
        QString receivedMagnet;
        int torrentSignals = 0;
        QObject::connect(view, &Content::ContentView::torrentReady, &observation, [&](const QString &magnet)
        {
            if (QThread::currentThread() != qApp->thread())
            {
                wrongThread.store(true);
                return;
            }
            receivedMagnet = magnet;
            ++torrentSignals;
        }, Qt::DirectConnection);
        controller->resolveVariant(selectedVariant);
        waitFor([&] { return !controller || !controller->property("resolving").toBool(); }, 50'000, u"Exact torrent resolution"_s);
        require(controller, u"Window closed during resolution"_s);
        require(controller->property("errorText").toString().isEmpty(),
            u"Resolution failed: %1"_s.arg(controller->property("errorText").toString()));
        require(torrentSignals == 1, u"Exact torrent resolution did not emit one native handoff"_s);
        const QUrl magnet(receivedMagnet);
        const QStringList hashes = QUrlQuery(magnet).allQueryItemValues(u"xt"_s);
        require((magnet.scheme() == u"magnet") && (hashes.size() == 1)
            && (hashes.first().compare(u"urn:btih:"_s + expectedHash, Qt::CaseInsensitive) == 0),
            u"Native handoff changed the selected torrent identity"_s);
        require((modelChanges > 0) && !wrongThread.load(), u"Model signal thread checks failed"_s);

        const int workCount = works->rowCount();
        const int variantCount = variants->rowCount();
        controller->search(u"One Piece"_s);
        require(controller->property("busy").toBool(), u"Shutdown scenario has no pending request"_s);
        bool closedWithPendingSearch = false;
        QTimer::singleShot(0, window, [&]
        {
            closedWithPendingSearch = controller && controller->property("busy").toBool();
            window->close();
        });
        waitFor([&] { return controller.isNull() && works.isNull() && variants.isNull(); }, 5'000, u"Close during search"_s);
        require(closedWithPendingSearch, u"Search completed before the shutdown scenario could close the window"_s);

        // Let already queued GUI callbacks run after their targets are deleted.
        QEventLoop drain;
        QTimer::singleShot(500, &drain, &QEventLoop::quit);
        drain.exec();
        require(!wrongThread.load(), u"A late model callback used the wrong thread"_s);
        qInfo().noquote() << u"PASS works=%1 variants=%2 cancel_restart=ok exact_magnet=ok close_pending=ok"_s
            .arg(workCount).arg(variantCount);
    }
}

int main(int argc, char *argv[])
{
    QApplication application(argc, argv);
    QCoreApplication::setApplicationName(u"qbutt-quick-acceptance"_s);
    QCoreApplication::setOrganizationName(u"qbutt-acceptance"_s);
    QStandardPaths::setTestModeEnabled(true);

    QCommandLineParser parser;
    parser.setApplicationDescription(u"Runs the production Qt Quick content view with an isolated application identity."_s);
    parser.addHelpOption();
    parser.addOption({u"exercise"_s, u"Run the bounded live metadata acceptance scenario and exit."_s});
    parser.addOption({u"dark"_s, u"Use the dark system color scheme for visual inspection."_s});
    parser.addOption({u"lang"_s, u"Set the locale for this process."_s, u"locale"_s, u"en"_s});
    parser.process(application);

    QLocale::setDefault(QLocale(parser.value(u"lang"_s)));
    if (parser.isSet(u"dark"_s))
        QGuiApplication::styleHints()->setColorScheme(Qt::ColorScheme::Dark);
    application.setQuitOnLastWindowClosed(!parser.isSet(u"exercise"_s));

    auto *window = new QMainWindow;
    window->setAttribute(Qt::WA_DeleteOnClose);
    window->setWindowTitle(u"qbutt — Qt Quick acceptance"_s);
    window->resize(1280, 760);
    auto *view = new Content::ContentView(window);
    window->setCentralWidget(view);
    QObject::connect(view, &Content::ContentView::torrentReady, window, [window](const QString &)
    {
        window->statusBar()->showMessage(u"Torrent handoff received; this acceptance host does not download files."_s, 5'000);
    });
    window->show();

    if (parser.isSet(u"exercise"_s))
    {
        QTimer::singleShot(180'000, &application, [&application]
        {
            qCritical("Acceptance exceeded its overall time limit");
            application.exit(1);
        });
        QTimer::singleShot(0, &application, [&application, window = QPointer(window), view = QPointer(view)]
        {
            try
            {
                exercise(window, view);
                application.exit(0);
            }
            catch (const std::exception &error)
            {
                qCritical().noquote() << "FAIL" << error.what();
                application.exit(1);
            }
        });
    }

    return application.exec();
}
