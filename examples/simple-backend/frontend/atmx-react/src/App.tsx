// FILE: src/App.tsx
import { useState } from "react";
import { sdk } from "./generated/sdk";
import { setAuthToken } from "atmx-react";
import * as models from "./generated/models";

function App() {
  const [email, setEmail] = useState("test@test.com");
  const [password, setPassword] = useState("123456");
  const [token, setToken] = useState<string | null>(null);

  // Keep track of a created item so we can fetch/delete it specifically
  const [createdItemId, setCreatedItemId] = useState<string | null>(null);

  // 1. Queries (Auto-fetches on mount)
  const {
    data: items,
    isLoading,
    error: listError,
    refetch,
  } = sdk.pyExample.useListItems({});
  const { data: singleItem } = sdk.pyExample.useGetItem(
    { itemId: createdItemId! },
    { enabled: !!createdItemId },
  );

  // 2. Mutations (Execute manually)
  const {
    execute: register,
    isMutating: isRegistering,
    error: regError,
  } = sdk.pyExample.useRegisterMutation();
  const {
    execute: login,
    isMutating: isLoggingIn,
    error: loginError,
  } = sdk.pyExample.useLoginMutation();
  const {
    execute: createItem,
    isMutating: isCreating,
    error: createError,
  } = sdk.pyExample.useCreateItemMutation();
  const { execute: deleteItem, isMutating: isDeleting } =
    sdk.pyExample.useDeleteItemMutation();

  const handleRegister = async () => {
    try {
      // Perfectly typed request body!
      const user = await register({ user: { email, password } });
      console.log("Registered User:", user);
      alert(`Registered successfully: ${user.id}`);
    } catch (e: any) {
      console.error("Register Error:", e);
    }
  };

  const handleLogin = async () => {
    try {
      const data = await login(
        { username: email, password },
        { headers: { "Content-Type": "application/x-www-form-urlencoded" } },
      );

      // ✨ FIX: Safe, automatically-namespaced token insertion!
      sdk.pyExample.setAuthToken("Authorization", data.accessToken);
      setToken(data.accessToken);

      alert("Logged in successfully! Token stored.");
      refetch();
    } catch (e: any) {
      console.error("Login Error:", e);
    }
  };

  const handleCreate = async () => {
    try {
      const newItem = await createItem({
        item: {
          title: "React Item",
          description: "Created from React ATMX SDK!",
        },
      });

      setCreatedItemId(newItem.id);
      alert("Item created successfully!");
      refetch(); // Instantly refresh the query list below!
    } catch (e: any) {
      console.error("Create Item Error:", e);
    }
  };

  const handleDelete = async () => {
    if (!createdItemId) return;

    try {
      await deleteItem({ itemId: createdItemId });
      setCreatedItemId(null);
      alert("Item deleted successfully!");
      refetch();
    } catch (e: any) {
      console.error("Delete Item Error:", e);
    }
  };

  return (
    <div
      style={{
        padding: 40,
        fontFamily: "system-ui",
        maxWidth: "800px",
        margin: "0 auto",
      }}
    >
      <h1 style={{ color: "#2563eb" }}>Axiom ATMX React</h1>

      {/* AUTHENTICATION */}
      <div
        style={{
          padding: 20,
          backgroundColor: "#f8fafc",
          borderRadius: 8,
          marginBottom: 20,
          border: "1px solid #e2e8f0",
        }}
      >
        <h2 style={{ marginTop: 0 }}>Authentication</h2>
        <div style={{ display: "flex", gap: 10, marginBottom: 20 }}>
          <input
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="Email"
            style={{ padding: 8, borderRadius: 4, border: "1px solid #cbd5e1" }}
          />
          <input
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            type="password"
            placeholder="Password"
            style={{ padding: 8, borderRadius: 4, border: "1px solid #cbd5e1" }}
          />
        </div>

        <div style={{ display: "flex", gap: 10 }}>
          <button
            onClick={handleRegister}
            disabled={isRegistering}
            style={{
              padding: "8px 16px",
              background: "#1e293b",
              color: "white",
              borderRadius: 4,
              cursor: isRegistering ? "not-allowed" : "pointer",
            }}
          >
            {isRegistering ? "Registering..." : "Register"}
          </button>

          <button
            onClick={handleLogin}
            disabled={isLoggingIn}
            style={{
              padding: "8px 16px",
              background: "#2563eb",
              color: "white",
              borderRadius: 4,
              cursor: isLoggingIn ? "not-allowed" : "pointer",
            }}
          >
            {isLoggingIn ? "Logging in..." : "Login"}
          </button>
        </div>

        {/* Display localized error messages gracefully */}
        {regError && (
          <p style={{ color: "red", fontSize: 12 }}>
            Register Error: {regError.message}
          </p>
        )}
        {loginError && (
          <p style={{ color: "red", fontSize: 12 }}>
            Login Error: {loginError.message}
          </p>
        )}
      </div>

      {/* SECURE ACTIONS */}
      <div
        style={{
          padding: 20,
          backgroundColor: "#f8fafc",
          borderRadius: 8,
          marginBottom: 20,
          border: "1px solid #e2e8f0",
        }}
      >
        <h2 style={{ marginTop: 0 }}>Secure Actions</h2>
        <p style={{ fontSize: 12, color: "#64748b" }}>
          Requires you to be logged in first.
        </p>

        <div style={{ display: "flex", gap: 10 }}>
          <button
            onClick={handleCreate}
            disabled={isCreating || !token}
            style={{
              padding: "8px 16px",
              background: "#16a34a",
              color: "white",
              borderRadius: 4,
              cursor: isCreating || !token ? "not-allowed" : "pointer",
            }}
          >
            {isCreating ? "Creating..." : "Create Item"}
          </button>

          <button
            onClick={handleDelete}
            disabled={isDeleting || !createdItemId}
            style={{
              padding: "8px 16px",
              background: "#dc2626",
              color: "white",
              borderRadius: 4,
              cursor: isDeleting || !createdItemId ? "not-allowed" : "pointer",
            }}
          >
            {isDeleting ? "Deleting..." : "Delete Last Item"}
          </button>
        </div>

        {createError && (
          <p style={{ color: "red", fontSize: 12 }}>
            Create Error: {createError.message}
          </p>
        )}
      </div>

      {/* REACTIVE DATA DISPLAY */}
      <div
        style={{
          padding: 20,
          backgroundColor: "#f8fafc",
          borderRadius: 8,
          border: "1px solid #e2e8f0",
        }}
      >
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
          }}
        >
          <h2 style={{ marginTop: 0 }}>Items List</h2>
          <button
            onClick={refetch}
            style={{
              fontSize: 12,
              color: "#2563eb",
              background: "none",
              border: "none",
              cursor: "pointer",
              textDecoration: "underline",
            }}
          >
            REFRESH
          </button>
        </div>

        {isLoading && (
          <p style={{ color: "#64748b" }}>Loading items from server...</p>
        )}
        {listError && (
          <p style={{ color: "red" }}>Error: {listError.message}</p>
        )}

        <ul style={{ paddingLeft: 20 }}>
          {/* ✨ MAGIC: Perfectly typed via models.pyExample.Item! */}
          {items?.map((item: models.pyExample.Item) => (
            <li key={item.id} style={{ marginBottom: 10 }}>
              <strong>{item.title}</strong>
              <div style={{ fontSize: 12, color: "#64748b" }}>
                {item.description}
              </div>
              <div style={{ fontSize: 10, color: "#94a3b8" }}>
                ID: {item.id}
              </div>
            </li>
          ))}
        </ul>

        {items?.length === 0 && !isLoading && (
          <p style={{ color: "#64748b", fontStyle: "italic" }}>
            No items found. Create one above!
          </p>
        )}
      </div>

      {/* SINGLE ITEM FETCH DISPLAY */}
      {singleItem && (
        <div
          style={{
            marginTop: 20,
            padding: 20,
            backgroundColor: "#f0fdf4",
            borderRadius: 8,
            border: "1px solid #bbf7d0",
          }}
        >
          <h2 style={{ marginTop: 0, color: "#16a34a" }}>
            Target Item Fetched!
          </h2>
          <p>
            We successfully ran a dedicated `useGetItem` query for the item you
            just created!
          </p>
          <strong>{singleItem.title}</strong>
        </div>
      )}
    </div>
  );
}

export default App;
