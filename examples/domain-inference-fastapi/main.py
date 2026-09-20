from typing import Annotated

from fastapi import FastAPI
from pydantic import BaseModel, Field

app = FastAPI()


class Customer(BaseModel):
    id: str
    email: Annotated[str, Field(min_length=3, max_length=320)]
    orders: list["Order"] = []


class Order(BaseModel):
    id: str
    customer: Customer
    total_cents: Annotated[int, Field(ge=0)]


Customer.model_rebuild()


@app.get("/customers/{customer_id}", response_model=Customer)
def get_customer(customer_id: str) -> Customer:
    return Customer(id=customer_id, email="dev@axiomcore.dev", orders=[])


@app.get("/orders/{order_id}", response_model=Order)
def get_order(order_id: str) -> Order:
    return Order(
        id=order_id,
        customer=Customer(id="cus_1", email="dev@axiomcore.dev", orders=[]),
        total_cents=1200,
    )
