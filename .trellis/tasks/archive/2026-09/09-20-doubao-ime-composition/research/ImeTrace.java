// Investigation-only JDI observer. Does not invoke methods or mutate the target VM.
// Compile with JDK 17 and attach through an adb JDWP forward to the disposable debug app.
import com.sun.jdi.*;
import com.sun.jdi.connect.AttachingConnector;
import com.sun.jdi.event.*;
import com.sun.jdi.request.*;
import java.time.Instant;
import java.util.*;

public class ImeTrace {
    static final String CONNECTION = "io.github.leonfox28.zterm.TerminalView$TerminalInputConnection";
    static final Set<String> METHODS = Set.of(
        "setComposingText", "finishComposingText", "commitText", "setComposingRegion",
        "setSelection", "deleteSurroundingText", "deleteSurroundingTextInCodePoints",
        "getTextBeforeCursor", "getTextAfterCursor", "getSelectedText", "getSurroundingText",
        "getExtractedText", "beginBatchEdit", "endBatchEdit", "sendKeyEvent", "closeConnection");

    static Value field(ObjectReference object, String name) {
        Field f = object.referenceType().fieldByName(name);
        return f == null ? null : object.getValue(f);
    }

    static String value(Value value) {
        if (value == null) return "null";
        if (value instanceof StringReference string) return quote(string.value());
        if (value instanceof ObjectReference object &&
                object.referenceType().name().equals("android.text.SpannableStringBuilder")) {
            ArrayReference chars = (ArrayReference) field(object, "mText");
            int gapStart = ((IntegerValue) field(object, "mGapStart")).value();
            int gapLength = ((IntegerValue) field(object, "mGapLength")).value();
            StringBuilder text = new StringBuilder();
            List<Value> data = chars.getValues();
            for (int i = 0; i < data.size(); i++) {
                if (i < gapStart || i >= gapStart + gapLength)
                    text.append(((CharValue) data.get(i)).value());
            }
            return quote(text.toString());
        }
        return value.toString();
    }

    static String quote(String string) {
        return "\"" + string.replace("\\", "\\\\").replace("\"", "\\\"")
            .replace("\n", "\\n").replace("\r", "\\r") + "\"";
    }

    static String state(ObjectReference connection) {
        ObjectReference view = (ObjectReference) field(connection, "this$0");
        ObjectReference editable = (ObjectReference) field(view, "composing");
        int count = ((IntegerValue) field(editable, "mSpanCount")).value();
        ArrayReference spans = (ArrayReference) field(editable, "mSpans");
        ArrayReference starts = (ArrayReference) field(editable, "mSpanStarts");
        ArrayReference ends = (ArrayReference) field(editable, "mSpanEnds");
        List<String> metadata = new ArrayList<>();
        for (int i = 0; i < count; i++) {
            Value span = spans.getValue(i);
            if (span instanceof ObjectReference object) {
                String name = object.referenceType().name();
                metadata.add(name + ":" + starts.getValue(i) + ".." + ends.getValue(i));
            }
        }
        return "buffer=" + value(editable) + " spans=" + metadata;
    }

    static String arguments(StackFrame frame) {
        try {
            return frame.getArgumentValues().stream().map(ImeTrace::value).toList().toString();
        } catch (RuntimeException unsupported) {
            try {
                List<String> values = new ArrayList<>();
                for (LocalVariable variable : frame.visibleVariables()) {
                    if (variable.isArgument()) {
                        try { values.add(variable.name() + "=" + value(frame.getValue(variable))); }
                        catch (RuntimeException unavailable) { values.add(variable.name() + "=<unavailable>"); }
                    }
                }
                return values.toString();
            } catch (AbsentInformationException | RuntimeException missing) { return "<unavailable>"; }
        }
    }

    public static void main(String[] args) throws Exception {
        AttachingConnector connector = Bootstrap.virtualMachineManager().attachingConnectors()
            .stream().filter(c -> c.name().equals("com.sun.jdi.SocketAttach")).findFirst().orElseThrow();
        var settings = connector.defaultArguments();
        settings.get("hostname").setValue("127.0.0.1");
        settings.get("port").setValue(args[0]);
        VirtualMachine vm = connector.attach(settings);
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            try { vm.dispose(); } catch (RuntimeException ignored) { }
        }));
        for (String name : List.of(CONNECTION, "android.view.inputmethod.BaseInputConnection")) {
            MethodEntryRequest entry = vm.eventRequestManager().createMethodEntryRequest();
            entry.addClassFilter(name);
            entry.setSuspendPolicy(EventRequest.SUSPEND_EVENT_THREAD);
            entry.enable();
            MethodExitRequest exit = vm.eventRequestManager().createMethodExitRequest();
            exit.addClassFilter(name);
            exit.setSuspendPolicy(EventRequest.SUSPEND_EVENT_THREAD);
            exit.enable();
        }
        System.out.println("ATTACHED " + Instant.now());
        vm.resume();
        while (true) {
            EventSet events = vm.eventQueue().remove();
            try {
                for (Event event : events) {
                    Method method;
                    ThreadReference thread;
                    boolean entry;
                    if (event instanceof MethodEntryEvent e) {
                        method = e.method(); thread = e.thread(); entry = true;
                    } else if (event instanceof MethodExitEvent e) {
                        method = e.method(); thread = e.thread(); entry = false;
                    } else { continue; }
                    if (!METHODS.contains(method.name())) continue;
                    StackFrame frame = thread.frame(0);
                    ObjectReference receiver = frame.thisObject();
                    if (receiver == null || !receiver.referenceType().name().equals(CONNECTION)) continue;
                    String details = entry
                        ? " args=" + arguments(frame)
                        : " result=" + value(((MethodExitEvent) event).returnValue());
                    System.out.println(Instant.now() + " " + (entry ? "ENTER " : "EXIT  ") +
                        method.name() + details + " " + state(receiver));
                }
            } finally { events.resume(); }
        }
    }
}
