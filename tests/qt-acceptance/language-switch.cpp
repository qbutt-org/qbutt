// SPDX-License-Identifier: GPL-2.0-or-later
#include "language-switch.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QAccessible>
#include <QAction>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QLibraryInfo>
#include <QLocale>
#include <QPushButton>
#include <QQuickItem>
#include <QTabWidget>
#include <QThread>

#include "app/application.h"
#include "base/bittorrent/session.h"
#include "base/net/portforwarder.h"
#include "base/preferences.h"
#include "gui/mainwindow.h"
#include "gui/optionsdialog.h"
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

    void waitFor(const QString &operation, const std::function<bool()> &ready, const int timeout = 5000)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < timeout))
        {
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
            QThread::msleep(1);
        }
        require(ready(), operation + u" timed out"_s);
    }

    template <typename T>
    T *child(QObject *parent, const QString &name)
    {
        auto *result = parent->findChild<T *>(name);
        require(result, u"Missing object: "_s + name);
        return result;
    }

    QString role(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        const auto roles = model->roleNames();
        for (auto it = roles.cbegin(); it != roles.cend(); ++it)
        {
            if (it.value() == name)
                return model->data(model->index(row, 0), it.key()).toString();
        }
        throw std::runtime_error("Missing model role: " + name.toStdString());
    }
}

void LanguageSwitchAcceptance::run(Application &application, const QJsonObject &spec, QJsonObject &evidence)
{
    const QString translations = QLibraryInfo::path(QLibraryInfo::TranslationsPath);
    require(QDir(translations).canonicalPath() == QDir(spec.value(u"bundle"_s).toString() + u"/translations").canonicalPath(),
        u"Qt translations do not come from the deployed bundle"_s);
    auto *session = BitTorrent::Session::instance();
    require(session && session->torrents().isEmpty() && (session->networkInterfaceAddress() == u"127.0.0.1")
        && !session->isDHTEnabled() && !session->isLSDEnabled() && !session->isPeXEnabled()
        && !Net::PortForwarder::instance()->isEnabled(), u"Language fixture is not isolated"_s);
    auto *window = application.mainWindow();
    const auto views = window->findChildren<Content::ContentView *>();
    require((views.size() == 1) && (window->currentTabWidget() == views.first()),
        u"MainWindow did not open its native content view"_s);
    auto *view = views.first();
    require(view->status() == QQuickWidget::Ready && view->rootObject(), u"Main QML did not load"_s);
    auto *root = view->rootObject();
    auto *controller = child<Qbutt::ContentController>(view, u"contentController"_s);
    window->resize(1440, 900);
    window->show();

    const QString phase = spec.value(u"phase"_s).toString();
    const bool startupEnglish = phase == u"restart-en";
    require((phase == u"switch") || startupEnglish || (phase == u"restart-ru"), u"Unknown language phase"_s);
    auto *optionsAction = child<QAction>(window, u"actionOptions"_s);
    optionsAction->trigger();
    auto *options = window->findChild<OptionsDialog *>();
    require(options && options->isVisible(), u"Main Options action did not open the dialog"_s);
    auto *language = child<QComboBox>(options, u"comboLanguage"_s);
    auto *buttons = child<QDialogButtonBox>(options, u"buttonBox"_s);
    auto *apply = buttons->button(QDialogButtonBox::Apply);
    require(apply, u"Missing native Apply action"_s);
    auto *query = child<QQuickItem>(root, u"contentQuery"_s);
    QJsonArray labels;

    const auto checkLabels = [&](const bool english)
    {
        const QString expectedLocale = english ? u"en"_s : u"ru"_s;
        require(Preferences::instance()->getLocale() == expectedLocale, u"Applied Locale does not match UI intent"_s);
        require(QLocale().language() == (english ? QLocale::English : QLocale::Russian), u"Default Qt locale stayed stale"_s);
        waitFor(u"Live Main and QML translation"_s, [&]
        {
            return (optionsAction->text() == MainWindow::tr("&Options..."))
                && (child<QQuickItem>(root, u"contentSearchMode"_s)->property("text").toString()
                    == QCoreApplication::translate("SearchPage", "Search"))
                && (query->property("placeholderText").toString() == QCoreApplication::translate("SearchPage", "Search titles"))
                && (child<QLabel>(options, u"labelLanguage"_s)->text() == OptionsDialog::tr("Language:"))
                && ((apply->text() == u"Apply") == english);
        });
        require((query->property("placeholderText").toString() == u"Search titles") == english,
            u"Actual QML label did not change language"_s);
        require((child<QLabel>(options, u"labelLanguage"_s)->text() == u"Language:") == english,
            u"Actual Options label did not change language"_s);
        const QList<QPair<QString, const char *>> actions {
            {u"actionOpenMedia"_s, "Open media…"}, {u"actionReadFile"_s, "Read file…"},
            {u"actionContentDownloads"_s, "Downloads"}, {u"actionImportProfile"_s, "Import profile..."},
            {u"actionCompletionPolicies"_s, "Completion policies…"}};
        for (const auto &[name, source] : actions)
            require(child<QAction>(window, name)->text() == MainWindow::tr(source), u"Main action stayed stale: "_s + name);
        auto *tabs = window->findChild<QTabWidget *>();
        require(tabs && (tabs->tabText(tabs->indexOf(view)) == MainWindow::tr("Search")), u"Main Search tab stayed stale"_s);
        labels.append(QJsonObject {{u"locale"_s, expectedLocale}, {u"options"_s, optionsAction->text()},
            {u"query"_s, query->property("placeholderText").toString()}, {u"apply"_s, apply->text()}});
    };
    checkLabels(startupEnglish);

    if (phase != u"restart-ru")
    {
        buttons->button(QDialogButtonBox::Cancel)->click();
        waitFor(u"Content library startup"_s, [&] { return controller->getLibraryState() == Qbutt::Library::LibraryState::Ready; });
        query->setProperty("text", u"Frieren"_s);
        controller->search(u"Frieren"_s);
        auto *works = qobject_cast<QAbstractItemModel *>(controller->property("works").value<QObject *>());
        auto *variants = qobject_cast<QAbstractItemModel *>(controller->property("variants").value<QObject *>());
        require(works && variants, u"Native models are missing"_s);
        waitFor(u"Anime search"_s, [&]
        {
            return (controller->getSearchState() == Qbutt::Search::SearchState::Succeeded) && (works->rowCount() > 0);
        }, 60000);
        const QString publication = u"aniliberty:9542"_s;
        bool found = false;
        for (int row = 0; row < works->rowCount(); ++row)
            found |= role(works, row, "workId") == publication;
        require(found, u"Search did not return the exact AniLiberty publication"_s);
        controller->selectWork(publication);
        waitFor(u"Anime variants"_s, [&] { return !controller->getInspecting() && (variants->rowCount() > 0); }, 60000);
        int selectedRow = -1;
        for (int row = 0; row < variants->rowCount(); ++row)
        {
            if (role(variants, row, "kind") == u"video")
            {
                selectedRow = row;
                break;
            }
        }
        require(selectedRow >= 0, u"Fixture has no video episode"_s);
        const QString variantId = role(variants, selectedRow, "variantId");
        controller->selectVariant(variantId);
        auto *list = child<QQuickItem>(root, u"contentVariants"_s);
        require(QMetaObject::invokeMethod(list, "positionViewAtIndex", Q_ARG(int, selectedRow), Q_ARG(int, 0)),
            u"Variant list positioning failed"_s);
        const QString workId = controller->getSelectedWorkId();
        const int variantCount = variants->rowCount();
        optionsAction->trigger();
        options = window->findChild<OptionsDialog *>();
        require(options, u"Options did not reopen"_s);
        language = child<QComboBox>(options, u"comboLanguage"_s);
        buttons = child<QDialogButtonBox>(options, u"buttonBox"_s);
        apply = buttons->button(QDialogButtonBox::Apply);
        const QStringList choices = startupEnglish ? QStringList {u"ru"_s} : QStringList {u"en"_s, u"ru"_s, u"en"_s};
        for (const QString &locale : choices)
        {
            const int index = language->findData(locale);
            require(index >= 0, u"Native locale choice is missing"_s);
            language->setCurrentIndex(index);
            require(apply->isEnabled(), u"Locale choice did not enable Apply"_s);
            apply->click();
            const bool english = locale == u"en";
            checkLabels(english);
            waitFor(u"Live native variant label"_s, [&]
            {
                const QString label = role(variants, selectedRow, "label");
                if (label.startsWith(u"Episode") != english)
                    return false;
                QQuickItem *item = nullptr;
                require(QMetaObject::invokeMethod(list, "itemAtIndex", Q_RETURN_ARG(QQuickItem *, item), Q_ARG(int, selectedRow)),
                    u"Variant delegate lookup failed"_s);
                if (!item)
                    return false;
                auto *accessible = QAccessible::queryAccessibleInterface(item);
                return (item->property("variantId").toString() == variantId)
                    && (item->property("label").toString() == label)
                    && accessible && (accessible->text(QAccessible::Name) == label)
                    && (child<QQuickItem>(root, u"contentDownload"_s)->property("text").toString()
                        == QCoreApplication::translate("WorkPane", "Watch"));
            });
            require((controller->getSelectedWorkId() == workId) && (controller->getSelectedVariantId() == variantId)
                && (variants->rowCount() == variantCount) && (query->property("text").toString() == u"Frieren")
                && (controller->getSearchState() == Qbutt::Search::SearchState::Succeeded)
                && !controller->getInspecting(), u"Language change altered content selection"_s);
        }
    }
    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    checks.append(QJsonObject {{u"name"_s, u"language-switch"_s}, {u"phase"_s, phase}, {u"labels"_s, labels},
        {u"savedLocale"_s, Preferences::instance()->getLocale()}, {u"torrentCount"_s, session->torrents().size()},
        {u"translations"_s, translations}});
    evidence[u"checks"_s] = checks;
}
