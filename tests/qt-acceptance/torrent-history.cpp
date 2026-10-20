// SPDX-License-Identifier: GPL-2.0-or-later
#include "torrent-history.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QCheckBox>
#include <QDialogButtonBox>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QPointer>
#include <QPushButton>
#include <QStringList>
#include <QThread>

#include "app/application.h"
#include "base/bittorrent/addtorrenterror.h"
#include "base/bittorrent/session.h"
#include "base/bittorrent/torrent.h"
#include "base/preferences.h"
#include "gui/addnewtorrentdialog.h"
#include "gui/guiaddtorrentmanager.h"
#include "gui/mainwindow.h"
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

    void waitFor(const QString &operation, const std::function<bool()> &ready)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 15'000))
        {
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
            QThread::msleep(1);
        }
        require(ready(), operation + u" timed out"_s);
    }

    QAbstractItemModel *model(Qbutt::ContentController &controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller.property(name).value<QObject *>());
        require(result, u"Missing production model: "_s + QString::fromLatin1(name));
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &role)
    {
        const int id = model->roleNames().key(role, -1);
        require(id >= 0, u"Missing production model role"_s);
        return model->data(model->index(row, 0), id).toString();
    }

    QList<AddNewTorrentDialog *> dialogs()
    {
        QList<AddNewTorrentDialog *> result;
        for (QWidget *widget : QApplication::topLevelWidgets())
        {
            if (auto *dialog = qobject_cast<AddNewTorrentDialog *>(widget); dialog && dialog->isVisible())
                result.append(dialog);
        }
        return result;
    }

    QPointer<AddNewTorrentDialog> resolvedDialog(Qbutt::ContentController &controller, const QString &variant)
    {
        controller.resolveVariant(variant);
        waitFor(u"Exact provider resolution"_s, [&] { return !controller.property("resolving").toBool(); });
        require(controller.property("errorText").toString().isEmpty(), controller.property("errorText").toString());
        waitFor(u"MainWindow torrent dialog"_s, [] { return !dialogs().isEmpty(); });
        require(dialogs().size() == 1, u"Expected one native torrent dialog"_s);
        return dialogs().first();
    }

    void clickDialog(AddNewTorrentDialog &dialog, const QDialogButtonBox::StandardButton action)
    {
        auto *buttons = dialog.findChild<QDialogButtonBox *>(u"buttonBox"_s);
        auto *button = buttons ? buttons->button(action) : nullptr;
        require(button && button->isEnabled(), u"Native dialog action is missing or disabled"_s);
        button->click();
    }

    void inspect(Qbutt::ContentController &controller, const QString &work)
    {
        controller.selectWork(work);
        waitFor(u"Publication inspection"_s, [&] { return !controller.property("inspecting").toBool(); });
        require(controller.property("selectedWorkId").toString() == work
            && controller.property("errorText").toString().isEmpty(), u"Cannot inspect exact fixture work"_s);
    }

    void assertHistory(Qbutt::ContentController &controller, const int count)
    {
        waitFor(u"Library work drain"_s, [&] { return !controller.property("libraryPending").toBool(); });
        require(controller.refreshHistory(), u"History refresh was rejected"_s);
        waitFor(u"History refresh"_s, [&] { return !controller.property("libraryPending").toBool(); });
        require(model(controller, "history")->rowCount() == count, u"History count differs from confirmed native additions"_s);
    }

    void reopenHistory(Qbutt::ContentController &controller, const QString &work, const QString &variant)
    {
        controller.setListMode(Qbutt::Search::ListMode::History);
        inspect(controller, work);
        require(controller.property("selectedVariantId").toString() == variant,
            u"History restored %1 instead of %2"_s.arg(controller.property("selectedVariantId").toString(), variant));
        controller.setListMode(Qbutt::Search::ListMode::Search);
    }
}

void TorrentHistoryAcceptance::run(Application &application, const QJsonObject &spec, QJsonObject &evidence)
{
    auto *window = application.mainWindow();
    window->hide();
    const auto views = window->findChildren<Content::ContentView *>();
    require(views.size() == 1, u"MainWindow must own the content controller"_s);
    auto *controller = views.first()->findChild<Qbutt::ContentController *>();
    require(controller, u"Missing MainWindow content controller"_s);
    waitFor(u"Content library"_s, [&]
    {
        return controller->getLibraryState() == Qbutt::Library::LibraryState::Ready
            && controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>()
                == Qbutt::Sources::SettingsState::Ready;
    });

    const QString magnetWork = u"aniliberty:9700"_s;
    const QString magnetVariant = magnetWork + u":torrent:9702"_s;
    const QString magnetPreference = magnetWork + u":torrent:9701"_s;
    const QString metadataWork = u"igruha:42-generated-history"_s;
    const QString metadataVariant = metadataWork + u":torrent:"_s + spec.value(u"metadataHash"_s).toString();
    QJsonArray checks;
    if (spec.value(u"restart"_s).toBool())
    {
        assertHistory(*controller, 2);
        reopenHistory(*controller, magnetWork, magnetVariant);
        reopenHistory(*controller, metadataWork, metadataVariant);
        checks.append(QJsonObject {{u"name"_s, u"torrent-history-restart"_s}, {u"exactVariants"_s, true}});
        evidence[u"checks"_s] = checks;
        return;
    }

    auto *session = BitTorrent::Session::instance();
    require(session->torrents().isEmpty() && !session->isDHTEnabled() && !session->isLSDEnabled()
            && !session->isPeXEnabled() && !session->isAddTrackersEnabled()
            && !session->isAddTrackersFromURLEnabled() && session->networkInterfaceAddress() == u"127.0.0.1",
        u"Torrent history requires an empty loopback-only session"_s);
    auto *manager = static_cast<IGUIApplication &>(application).addTorrentManager();
    int added = 0;
    int sessionAdded = 0;
    int dismissed = 0;
    int failures = 0;
    int magnetHandoffs = 0;
    int metadataHandoffs = 0;
    QStringList attemptSources;
    QObject observation;
    QObject::connect(session, &BitTorrent::Session::torrentAdded, &observation,
        [&](BitTorrent::Torrent *torrent)
    {
        const QString hash = torrent->infoHash().v1().toString();
        require(hash == spec.value(u"magnetHash"_s).toString()
                || hash == spec.value(u"metadataHash"_s).toString(),
            u"libtorrent added an unrelated torrent"_s);
        ++sessionAdded;
    });
    QObject::connect(manager, &AddTorrentManager::torrentAdded, &observation,
        [&](const QString &source, BitTorrent::Torrent *)
    {
        require(attemptSources.contains(source), u"Native addition lost its resolve attempt identity"_s);
        ++added;
    });
    QObject::connect(manager, &GUIAddTorrentManager::addTorrentDismissed, &observation, [&] { ++dismissed; });
    QObject::connect(manager, &AddTorrentManager::addTorrentFailed, &observation, [&] { ++failures; });
    QObject::connect(views.first(), &Content::ContentView::torrentReady, &observation,
        [&](const QString &source, const QString &)
    {
        require(source.startsWith(u"qbutt-content:") && !attemptSources.contains(source),
            u"Torrent attempts reused an identity"_s);
        attemptSources.append(source);
        ++magnetHandoffs;
    });
    QObject::connect(views.first(), &Content::ContentView::torrentMetadataReady, &observation,
        [&](const QString &source, const QByteArray &)
    {
        require(source.startsWith(u"qbutt-content:") && !attemptSources.contains(source),
            u"Metadata attempts reused an identity"_s);
        attemptSources.append(source);
        ++metadataHandoffs;
    });
    auto *sources = model(*controller, "sources");
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const QString source = value(sources, row, "sourceId");
        require(controller->setSourceEnabled(source, source == u"aniliberty" || source == u"igruha"),
            u"Cannot isolate generated providers"_s);
    }
    waitFor(u"Source settings writes"_s, [&] { return !controller->property("libraryPending").toBool(); });
    controller->search(u"Generated history"_s);
    require(controller->property("busy").toBool(),
        u"Generated search did not start: "_s + controller->property("errorText").toString());
    waitFor(u"Generated provider search"_s, [&] { return !controller->property("busy").toBool(); });
    QStringList sourceErrors;
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        if (value(sources, row, "failed") == u"true")
            sourceErrors.append(value(sources, row, "sourceId") + u": "_s + value(sources, row, "failureText"));
    }
    require(model(*controller, "works")->rowCount() == 2,
        u"Generated search returned %1 works: %2; %3"_s.arg(model(*controller, "works")->rowCount())
            .arg(controller->property("errorText").toString(), sourceErrors.join(u"; "_s)));
    inspect(*controller, magnetWork);
    require(model(*controller, "variants")->rowCount() == 2, u"Exact same-hash variants are missing"_s);
    controller->selectVariant(magnetVariant);
    auto dialog = resolvedDialog(*controller, magnetVariant);
    assertHistory(*controller, 0);
    clickDialog(*dialog, QDialogButtonBox::Cancel);
    waitFor(u"Cancelled dialog retirement"_s, [&] { return !dialog && dismissed == 1; });
    assertHistory(*controller, 0);
    require(session->torrents().isEmpty(), u"Cancelled torrent entered the session"_s);
    checks.append(QJsonObject {{u"name"_s, u"cancel-keeps-history-empty"_s}, {u"passed"_s, true}});

    dialog = resolvedDialog(*controller, magnetVariant);
    controller->selectVariant(magnetPreference);
    auto repeated = resolvedDialog(*controller, magnetPreference);
    waitFor(u"Repeated same-hash attempt dismissal"_s, [&] { return dismissed == 2; });
    require(repeated == dialog, u"Same hash opened a second native dialog"_s);
    assertHistory(*controller, 0);
    inspect(*controller, metadataWork);
    auto *start = dialog->findChild<QCheckBox *>(u"startTorrentCheckBox"_s);
    require(start && !start->isChecked(), u"Generated torrent must be added stopped"_s);
    clickDialog(*dialog, QDialogButtonBox::Ok);
    waitFor(u"libtorrent magnet addition"_s, [&] { return !dialog && added == 1; });
    assertHistory(*controller, 1);
    require(session->torrents().size() == 1 && session->torrents().first()->isStopped(),
        u"Magnet acceptance did not produce one stopped native torrent"_s);
    checks.append(QJsonObject {{u"name"_s, u"same-hash-keeps-original-intent"_s},
        {u"selectionChangedBeforeAdd"_s, true}, {u"exactVariant"_s, magnetVariant}});

    inspect(*controller, metadataWork);
    require(model(*controller, "variants")->rowCount() == 2, u"Generated metadata variants are missing"_s);
    controller->selectVariant(metadataVariant);
    QFile fault(spec.value(u"faultPath"_s).toString());
    require(fault.open(QIODevice::WriteOnly) && fault.write("malformed") == 9,
        u"Cannot arm malformed metadata fixture"_s);
    fault.close();
    controller->resolveVariant(metadataVariant);
    waitFor(u"Malformed provider metadata rejection"_s, [&] { return !controller->property("resolving").toBool(); });
    bool metadataFailure = false;
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        if (value(sources, row, "sourceId") == u"igruha")
        {
            metadataFailure = (value(sources, row, "failed") == u"true")
                && !value(sources, row, "failureText").isEmpty();
        }
    }
    require(metadataFailure && dialogs().isEmpty() && metadataHandoffs == 0,
        u"Malformed metadata was not rejected by the source"_s);
    assertHistory(*controller, 1);
    require(fault.remove(), u"Cannot restore valid metadata fixture"_s);

    const qint64 sizeLimit = Preferences::instance()->getTorrentFileSizeLimit();
    Preferences::instance()->setTorrentFileSizeLimit(1);
    controller->resolveVariant(metadataVariant);
    waitFor(u"Native metadata handoff failure"_s, [&] { return failures == 1; });
    Preferences::instance()->setTorrentFileSizeLimit(sizeLimit);
    require(dialogs().isEmpty(), u"Rejected metadata opened a torrent dialog"_s);
    assertHistory(*controller, 1);
    checks.append(QJsonObject {{u"name"_s, u"failed-metadata-keeps-history"_s}, {u"passed"_s, true}});

    dialog = resolvedDialog(*controller, metadataVariant);
    auto *hash = dialog->findChild<QLabel *>(u"labelInfohash1Data"_s);
    require(hash && hash->text().compare(spec.value(u"metadataHash"_s).toString(), Qt::CaseInsensitive) == 0,
        u"Metadata dialog substituted the selected torrent"_s);
    assertHistory(*controller, 1);
    inspect(*controller, magnetWork);
    clickDialog(*dialog, QDialogButtonBox::Ok);
    waitFor(u"libtorrent metadata addition"_s, [&] { return !dialog && added == 2; });
    assertHistory(*controller, 2);
    reopenHistory(*controller, metadataWork, metadataVariant);
    require(magnetHandoffs == 3 && metadataHandoffs == 2 && failures == 1 && sessionAdded == 2
            && session->torrents().size() == 2,
        u"Native outcome accounting differs from the generated actions"_s);
    checks.append(QJsonObject {{u"name"_s, u"metadata-add-records-exact-history"_s},
        {u"exactVariant"_s, metadataVariant}, {u"nativeAdditions"_s, added},
        {u"sessionAdditions"_s, sessionAdded},
        {u"magnetHandoffs"_s, magnetHandoffs}, {u"metadataHandoffs"_s, metadataHandoffs}});
    evidence[u"checks"_s] = checks;
}
